use super::ChildError;
use std::io;

/// 保留原进程组终止与Darwin退出过渡处理，不取得新PID所有权。
/// 来源：原生 Rust UnixChild 原 terminate_group 逻辑；无 Java 对等对象。
pub(super) struct UnixChildGroup;

impl UnixChildGroup {
    /// 单次终止原保留组，不等待 Darwin 退出过渡，不在期限内 sleep/retry。
    /// 参数：pid 为仍保有等待权的原正 leader；返回：已发送/全 zombie 为 true，退出过渡为 false。
    /// false 必须保留原 owner 继续轮询，未知或权限拒绝仍返回原错误。
    #[cfg(target_os = "macos")]
    pub(super) fn terminate_once(pid: i32) -> Result<bool, ChildError> {
        #[cfg(test)]
        if super::unix_normal_exit_tests::group_termination_failure() {
            return Err(ChildError::io(
                "injected owned process group termination failure",
                io::Error::from_raw_os_error(libc::EPERM),
            ));
        }
        if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH)
            || (error.raw_os_error() == Some(libc::EPERM)
                && super::macos_child_group::zombies_only(pid as u32))
        {
            return Ok(true);
        }
        if error.raw_os_error() == Some(libc::EPERM)
            && super::macos_child_group::exiting_only(pid as u32)
        {
            return Ok(false);
        }
        Err(ChildError::io("terminate owned process group", error))
    }

    /// 终止原保留组。参数：pid为尚未wait的原leader；返回：原OS错误与原有有界重试。
    pub(super) fn terminate(pid: i32) -> Result<(), ChildError> {
        #[cfg(test)]
        if super::unix_normal_exit_tests::group_termination_failure() {
            return Err(ChildError::io(
                "injected owned process group termination failure",
                io::Error::from_raw_os_error(libc::EPERM),
            ));
        }
        if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if error.raw_os_error() == Some(libc::EPERM) {
            return retry_darwin_group(pid);
        }
        Err(ChildError::io("terminate owned process group", error))
    }
}
#[cfg(target_os = "macos")]
fn retry_darwin_group(pid: i32) -> Result<(), ChildError> {
    let retry_until = std::time::Instant::now() + std::time::Duration::from_millis(50);
    loop {
        if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        if error.raw_os_error() == Some(libc::EPERM) {
            // Darwin 退出过渡可能仍为 SRUN/INEXIT。仅最终稳定 SZOMB 证明
            // 可以消除该错误；未知或真实权限错误在有界重试后仍明确失败。
            if super::macos_child_group::zombies_only(pid as u32) {
                return Ok(());
            }
            if std::time::Instant::now() < retry_until {
                std::thread::sleep(std::time::Duration::from_millis(2));
                continue;
            }
        }
        return Err(ChildError::io("terminate owned process group", error));
    }
}
