use super::{LinuxSupervisorTrust, SlotError, SlotReservation};
use std::ffi::CString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};
use std::time::Instant;

/// Linux 宿主预置的稳定监督状态域。来源：PF-06；无 Java 对等对象。
/// 仅 root 可替换祖先/目录，槽由当前服务 UID 独占写入；不创建、修复或签发部署信任。
/// 这是准入组件，尚不构成监督出生、私有 IPC 或有限前端退出实现。
pub struct LinuxSupervisorNamespace {
    directories: Vec<File>,
    trust: LinuxSupervisorTrust,
    service_uid: libc::uid_t,
}

impl LinuxSupervisorNamespace {
    /// 打开可信宿主预置域。参数：root/trust 为宿主原材料，deadline 为原期限。
    /// frontend_uid 必须由 root broker 从原连接内核凭据取得，不得来自客户声明。
    /// 返回：逐组件 no-follow 的原目录链；远程参数不得作为 root 的来源。
    /// 所有祖先必须 root 所有且不可由组/其他用户写入，不接受普通 0700 用户目录。
    pub fn from_host(
        root: &Path,
        trust: LinuxSupervisorTrust,
        frontend_uid: libc::uid_t,
        deadline: Instant,
    ) -> Result<Self, SlotError> {
        super::slot_reservation::check(deadline)?;
        let service_uid = unsafe { libc::geteuid() };
        // 目录不可替换不等于槽内容不可伪造；同 UID 前端仍可写服务的 0600 文件。
        if service_uid == frontend_uid {
            return Err(SlotError::Unsupported);
        }
        verify_namespace(&trust, deadline)?;
        if !root.is_absolute() || root.as_os_str().as_bytes().len() > 1024 {
            return Err(SlotError::Unsupported);
        }
        let slash = c"/";
        let raw = unsafe {
            libc::open(
                slash.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut directories = vec![unsafe { File::from_raw_fd(raw) }];
        verify_directory(&directories[0], deadline)?;
        for component in root.components() {
            if component == Component::RootDir {
                continue;
            }
            let Component::Normal(name) = component else {
                return Err(SlotError::Unsupported);
            };
            super::slot_reservation::check(deadline)?;
            let name = CString::new(name.as_bytes()).map_err(|_| SlotError::Unsupported)?;
            let parent = directories.last().expect("held root");
            let raw = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if raw < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let directory = unsafe { File::from_raw_fd(raw) };
            verify_directory(&directory, deadline)?;
            directories.push(directory);
        }
        if directories.len() == 1 {
            return Err(SlotError::Unsupported);
        }
        let identity = directories.last().expect("held namespace").metadata()?;
        if (identity.dev(), identity.ino()) != super::linux_supervisor_trust::identity(&trust.root)?
        {
            return Err(SlotError::Unsupported);
        }
        verify_namespace(&trust, deadline)?;
        if unsafe { libc::geteuid() } != service_uid {
            return Err(SlotError::Unsupported);
        }
        Ok(Self {
            directories,
            trust,
            service_uid,
        })
    }

    /// 认领本服务 UID 的固定槽。参数：原期限；返回：同一预置普通文件的真实锁预留。
    /// 名称不接受请求输入；缺失、链接、多链接、错误 owner/模式和未确认记录均拒绝。
    /// 调用方须在监督整个寿命保留本域，不能把预留移交视为 Windows 的锁转移。
    pub fn reserve(&self, deadline: Instant) -> Result<SlotReservation, SlotError> {
        if unsafe { libc::geteuid() } != self.service_uid {
            return Err(SlotError::Unsupported);
        }
        verify_namespace(&self.trust, deadline)?;
        for directory in &self.directories {
            verify_directory(directory, deadline)?;
        }
        let uid = self.service_uid;
        let name = CString::new(format!("uid_{uid}.slot")).expect("numeric fixed slot name");
        let root = self.directories.last().expect("held namespace");
        let raw = unsafe {
            libc::openat(
                root.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(raw) };
        let metadata = file.metadata()?;
        super::slot_reservation::check(deadline)?;
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(SlotError::Unsupported);
        }
        verify_namespace(&self.trust, deadline)?;
        let reservation = SlotReservation::acquire(file, deadline)?;
        // 认领期间角色改变也不得返回原服务能力；失败保留已同步的未确认预留。
        if unsafe { libc::geteuid() } != self.service_uid {
            return Err(SlotError::Unsupported);
        }
        Ok(reservation)
    }
}

fn verify_namespace(trust: &LinuxSupervisorTrust, deadline: Instant) -> Result<(), SlotError> {
    for (path, expected) in [
        ("/proc/thread-self/ns/user", &trust.user_namespace),
        ("/proc/thread-self/ns/mnt", &trust.mount_namespace),
    ] {
        super::slot_reservation::check(deadline)?;
        let file = File::open(path)?;
        let mut state = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(file.as_raw_fd(), state.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let state = unsafe { state.assume_init() };
        let metadata = file.metadata()?;
        if state.f_type != 0x6e736673
            || (metadata.dev(), metadata.ino())
                != super::linux_supervisor_trust::identity(expected)?
        {
            return Err(SlotError::Unsupported);
        }
        super::slot_reservation::check(deadline)?;
    }
    Ok(())
}

fn verify_directory(file: &File, deadline: Instant) -> Result<(), SlotError> {
    super::slot_reservation::check(deadline)?;
    let metadata = file.metadata()?;
    super::slot_reservation::check(deadline)?;
    if !metadata.is_dir()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.nlink() == 0
    {
        return Err(SlotError::Unsupported);
    }
    Ok(())
}
