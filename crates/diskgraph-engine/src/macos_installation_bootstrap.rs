use crate::EngineError;
use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_installation_files::MacosInstallationFiles;
use crate::macos_installation_lease::open_namespace;
use diskgraph_core::BusinessError;
use std::ffi::CStr;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::time::Instant;

/// 首次可信安装的固定目录引导，不接受路径参数、不改写已有inode的权限或内容。
/// 来源：原生Rust PF-06永久安装锁合同，无Java对等对象。
pub(super) struct MacosInstallationBootstrap;
impl MacosInstallationBootstrap {
    /// 参数：原期限及取消检查点；返回：已持久建立或验证的永久保护布局。
    /// 只有真实和有效root身份均成立才允许创建，已有链接/不安全ACL拒绝。
    pub(super) fn prepare(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        check(deadline, checkpoint)?;
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return Err(BusinessError::Unsupported.into());
        }
        let root = open_at(libc::AT_FDCWD, c"/", libc::O_DIRECTORY)?;
        let state = MacosFilesystemState::capture(&root, true)?;
        let mut directories = vec![(root, state)];
        for (index, name) in [
            c"Library",
            c"Application Support",
            c"DiskGraph",
            c"scan-worker",
        ]
        .iter()
        .enumerate()
        {
            check(deadline, checkpoint)?;
            let parent = directories
                .last()
                .expect("root directory is retained")
                .0
                .as_raw_fd();
            let directory = match open_at(parent, name, libc::O_DIRECTORY) {
                Ok(file) => file,
                Err(EngineError::Io(error))
                    if error.raw_os_error() == Some(libc::ENOENT) && index >= 2 =>
                {
                    // 仅固定后两个产品目录允许创建；上级缺失不能自行猜测系统布局。
                    let made = unsafe { libc::mkdirat(parent, name.as_ptr(), 0o755) };
                    if made != 0 {
                        let error = std::io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::EEXIST) {
                            return Err(error.into());
                        }
                    }
                    let file = open_at(parent, name, libc::O_DIRECTORY)?;
                    if made == 0 && unsafe { libc::fchmod(file.as_raw_fd(), 0o755) } != 0 {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    file
                }
                Err(error) => return Err(error),
            };
            let state = MacosFilesystemState::capture(&directory, true)?;
            if state.mode & 0o005 != 0o005 {
                return Err(BusinessError::Unsupported.into());
            }
            directories.push((directory, state));
        }
        check(deadline, checkpoint)?;
        let parent = &directories.last().expect("fixed base retained").0;
        // 独占新锁inode，已有锁只重新打开并验证，绝不truncate/unlink/chmod已有锁。
        let writer = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                c"installation.lock".as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o444,
            )
        };
        if writer >= 0 {
            let writer = unsafe { File::from_raw_fd(writer) };
            if unsafe { libc::fchmod(writer.as_raw_fd(), 0o444) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            writer.sync_all()?;
            drop(writer);
        } else {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EEXIST) {
                return Err(error.into());
            }
        }
        let lock = open_at(parent.as_raw_fd(), c"installation.lock", libc::O_NONBLOCK)?;
        let state = MacosFilesystemState::capture(&lock, false)?;
        if state.mode & 0o004 == 0 {
            return Err(BusinessError::Unsupported.into());
        }
        if state.len != 0 {
            return Err(BusinessError::Conflict.into());
        }
        MacosInstallationFiles::durable(
            &lock,
            &[&directories[2].0, &directories[3].0, &directories[4].0],
            deadline,
            checkpoint,
        )?;
        for (file, old) in &directories {
            check(deadline, checkpoint)?;
            let current = MacosFilesystemState::capture(file, true)?;
            if !old.same_directory_binding(&current) {
                return Err(BusinessError::Conflict.into());
            }
        }
        let (_, _, current) = open_namespace(
            b"/Library/Application Support/DiskGraph/scan-worker/installation.lock",
            deadline,
            checkpoint,
        )?;
        if current != state {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, checkpoint)
    }
}
fn open_at(parent: libc::c_int, name: &CStr, extra: libc::c_int) -> Result<File, EngineError> {
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | extra,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // 每个成功返回的fd立即转唯一File owner；错误路径由局部owner释放。
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
