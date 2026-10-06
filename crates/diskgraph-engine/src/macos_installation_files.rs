use crate::EngineError;
use diskgraph_core::BusinessError;
use std::ffi::CStr;
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

/// 可信发行使用的原目录FD文件原语，不自行授予root来源或安装资格。
/// 来源：Apple SDK openat/renameat及fcntl(2) F_FULLFSYNC；无Java对等对象。
pub(super) struct MacosInstallationFiles;

impl MacosInstallationFiles {
    /// 参数：原父目录与单个固定名称；返回：独占新目录，不复用已有目标。
    pub(super) fn create_directory(parent: &File, name: &CStr) -> Result<File, EngineError> {
        component(name)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let directory = Self::open_directory(parent, name)?;
        set_mode(&directory, 0o755)?;
        Ok(directory)
    }

    /// 参数：原父目录与单个名称；返回：NOFOLLOW目录句柄，不跟随路径别名。
    pub(super) fn open_directory(parent: &File, name: &CStr) -> Result<File, EngineError> {
        open_at(parent, name, libc::O_RDONLY | libc::O_DIRECTORY)
    }

    /// 参数：原目录和单个名称；返回：独占新建0600写句柄；绝不chown或复用旧inode。
    pub(super) fn fresh_writer(parent: &File, name: &CStr) -> Result<File, EngineError> {
        open_at(parent, name, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)
    }

    /// 参数：原目录和单个名称；返回：原生只读非阻塞句柄，不导出执行权限。
    pub(super) fn read_only(parent: &File, name: &CStr) -> Result<File, EngineError> {
        open_at(parent, name, libc::O_RDONLY | libc::O_NONBLOCK)
    }

    /// 参数：最多64KiB材料、最终只读模式及原期限；返回：关闭唯一writer后的只读句柄。
    pub(super) fn write_fresh(
        parent: &File,
        name: &CStr,
        bytes: &[u8],
        mode: libc::mode_t,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<File, EngineError> {
        Self::check(deadline, checkpoint)?;
        if bytes.len() > 64 * 1024 {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let mut writer = Self::fresh_writer(parent, name)?;
        writer.write_all(bytes)?;
        Self::finish_writer(writer, mode, deadline, checkpoint)?;
        Self::read_only(parent, name)
    }

    /// 参数：本次独占writer及最终模式；返回：文件同步且唯一写FD已关闭的事实。
    /// 不重试close的EINTR，避免错误地关闭已被重用的数字FD；错误不允许后续签发。
    pub(super) fn finish_writer(
        writer: File,
        mode: libc::mode_t,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        Self::check(deadline, checkpoint)?;
        if !matches!(mode, 0o444 | 0o555) {
            return Err(BusinessError::InvalidArgument.into());
        }
        set_mode(&writer, mode)?;
        Self::durable(&writer, &[], deadline, checkpoint)?;
        let raw = writer.into_raw_fd();
        if unsafe { libc::close(raw) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Self::check(deadline, checkpoint)
    }

    /// 参数：regular锚点、同设备待同步目录、原期限；返回：真实持久化或原系统错误。
    /// SDK fcntl(2)说明FULLFSYNC使此前同设备已fsync数据持久化；不能降级为普通fsync。
    /// 设备自身违反flush合同仍不在软件证明内，真实断电与卷资格必须独立验收。
    pub(super) fn durable(
        anchor: &File,
        directories: &[&File],
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        Self::check(deadline, checkpoint)?;
        let metadata = anchor.metadata()?;
        if !metadata.is_file() {
            return Err(BusinessError::Unsupported.into());
        }
        // 先验证全部对象，再发持久化请求；跨设备不能用一个flush冒充完整阶段。
        for directory in directories {
            let current = directory.metadata()?;
            if !current.is_dir() || current.dev() != metadata.dev() {
                return Err(BusinessError::Unsupported.into());
            }
        }
        for directory in directories {
            Self::check(deadline, checkpoint)?;
            directory.sync_all()?;
        }
        Self::check(deadline, checkpoint)?;
        anchor.sync_all()?;
        Self::check(deadline, checkpoint)?;
        if unsafe { libc::fcntl(anchor.as_raw_fd(), libc::F_FULLFSYNC) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Self::check(deadline, checkpoint)
    }
    /// 参数：base/stage/anchor 为原目录与同卷锚点，deadline/checkpoint 为原期限；返回：下限及活跃配置发布结果，错误不回滚已提交下限。
    ///
    /// 参数：保护根、新版本目录、同卷锚点及原期限；先持久下限，再发布活跃配置。
    /// 任意错误保留已提交下限及残留新材料，不unlink、不回滚、不删除旧版本。
    pub(super) fn publish_pair(
        base: &File,
        stage: &File,
        anchor: &File,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        Self::durable(anchor, &[stage, base], deadline, checkpoint)?;
        rename(
            stage,
            c"epoch-floor.pending.json",
            base,
            c"epoch-floor.json",
        )?;
        Self::durable(anchor, &[stage, base], deadline, checkpoint)?;
        Self::complete_active(base, stage, anchor, deadline, checkpoint)
    }
    /// 参数：base/stage/anchor 为已认证恢复材料与锚点，deadline/checkpoint 为原期限；返回：活跃配置发布与耐久同步结果，不修改下限。
    ///
    /// 参数：已由独立floor认证的pending目录和同卷锚点；只完成active发布及同步。
    /// 本原语不验证信任；调用方须保留独占安装锁并核验绑定，绝不修改floor。
    pub(super) fn complete_active(
        base: &File,
        stage: &File,
        anchor: &File,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        Self::check(deadline, checkpoint)?;
        rename(stage, c"active.pending.json", base, c"active.json")?;
        Self::durable(anchor, &[stage, base], deadline, checkpoint)
    }

    /// 参数：原绝对期限和检查点；返回：未经替换的原取消或期限错误。
    pub(super) fn check(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(())
    }
}

fn open_at(parent: &File, name: &CStr, flags: libc::c_int) -> Result<File, EngineError> {
    component(name)?;
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { File::from_raw_fd(raw) })
}
fn component(name: &CStr) -> Result<(), EngineError> {
    let name = name.to_bytes();
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        return Err(BusinessError::InvalidArgument.into());
    }
    Ok(())
}
fn set_mode(file: &File, mode: libc::mode_t) -> Result<(), EngineError> {
    if unsafe { libc::fchmod(file.as_raw_fd(), mode) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn rename(from: &File, old: &CStr, to: &File, new: &CStr) -> Result<(), EngineError> {
    if unsafe { libc::renameat(from.as_raw_fd(), old.as_ptr(), to.as_raw_fd(), new.as_ptr()) } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
