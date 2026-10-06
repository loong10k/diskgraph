use crate::EngineError;
use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_installation_lease::open_namespace;
use diskgraph_core::BusinessError;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

const INSTALLATION_LOCK: &[u8] =
    b"/Library/Application Support/DiskGraph/scan-worker/installation.lock";

/// 永久root保护安装锁的独占FD守卫，协调可信更新者与扫描出生。
/// 来源：Darwin flock 与原生 Rust PF-06；无 Java 对等对象。
/// 不复制FD、不隐式创建锁文件；守卫结束时显式解锁，再关闭File，不提供认证锁服务。
pub(super) struct MacosInstallationLock {
    _file: File,
    _ancestors: Vec<(File, MacosFilesystemState)>,
}

impl Drop for MacosInstallationLock {
    fn drop(&mut self) {
        // 出生临界区已经结束；显式释放同一打开文件描述的锁，避免内核存续引用延长锁期。
        // 锁只由本守卫取得，不扩大更新者权限；单次非阻塞操作失败仍由最终 close 兜底。
        if unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN | libc::LOCK_NB) } != 0 {
            eprintln!("macOS installation lock explicit release failed");
        }
    }
}

impl MacosInstallationLock {
    /// 参数：无；返回：仅测试模拟出生时内核仍持有的同一打开文件描述，不用于产品准入。
    #[cfg(test)]
    pub(super) fn duplicate_for_test(&self) -> std::io::Result<File> {
        self._file.try_clone()
    }
    /// 参数：期限/检查点沿原请求；返回：原永久inode的共享锁守卫或原错误。
    /// 缺失/替换锁文件失败关闭，等待不持数据库、registry或本进程出生门。
    pub(super) fn acquire_shared(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        Self::acquire(libc::LOCK_SH, deadline, checkpoint)
    }

    /// 参数：原期限/检查点；返回：仅root可信更新者取得的同永久inode独占守卫。
    /// 不创建/替换锁文件；调用方不能用此入口改写任意用户选择的配置路径。
    pub(super) fn acquire_exclusive(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check(deadline, checkpoint)?;
        // 真实与有效身份都必须是root；不接受setuid提升的普通调用者。
        if unsafe { libc::getuid() } != 0 || unsafe { libc::geteuid() } != 0 {
            return Err(BusinessError::Unsupported.into());
        }
        Self::acquire(libc::LOCK_EX, deadline, checkpoint)
    }

    fn acquire(
        operation: libc::c_int,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let (ancestors, file, state) = open_namespace(INSTALLATION_LOCK, deadline, checkpoint)?;
        if state.len != 0 {
            return Err(BusinessError::Conflict.into());
        }
        // 在取得内核锁之前建立唯一守卫；后续检查失败或 panic 也必须显式解锁。
        let guard = Self {
            _file: file,
            _ancestors: ancestors,
        };
        wait_lock(&guard._file, operation, deadline, checkpoint)?;
        let held = MacosFilesystemState::capture(&guard._file, false)?;
        check(deadline, checkpoint)?;
        if held != state {
            return Err(BusinessError::Conflict.into());
        }
        let (current, _comparison, current_state) =
            open_namespace(INSTALLATION_LOCK, deadline, checkpoint)?;
        if current_state != state
            || current.len() != guard._ancestors.len()
            || current
                .iter()
                .zip(&guard._ancestors)
                .any(|((_, now), (_, original))| !original.same_directory_binding(now))
        {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, checkpoint)?;
        Ok(guard)
    }
    /// 参数：file 为隔离夹具文件，deadline/checkpoint 为原期限；返回：实际共享锁守卫或原错误，不赋予生产安装资格。
    ///
    /// 测试专用普通文件入口，仅验证实际内核竞争与守卫释放，不冒充root安全准入。
    #[cfg(test)]
    pub(super) fn test_shared(
        file: File,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let guard = Self {
            _file: file,
            _ancestors: Vec::new(),
        };
        wait_shared(&guard._file, deadline, checkpoint)?;
        Ok(guard)
    }
}
#[cfg(test)]
fn wait_shared(
    file: &File,
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    wait_lock(file, libc::LOCK_SH, deadline, checkpoint)
}
fn wait_lock(
    file: &File,
    operation: libc::c_int,
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    loop {
        check(deadline, checkpoint)?;
        // 原独立open的FD保活，非阻塞锁不进入不可取消的内核等待。
        if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } == 0 {
            check(deadline, checkpoint)?;
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK)
            && error.raw_os_error() != Some(libc::EINTR)
        {
            return Err(error.into());
        }
        check(deadline, checkpoint)?;
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(2)),
        );
    }
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
