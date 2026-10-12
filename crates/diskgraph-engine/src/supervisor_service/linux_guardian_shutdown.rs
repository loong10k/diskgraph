use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

// 单连接独立进程专用；handler只写lock-free标记，不触碰原owner、FD、堆或日志。
static STOPPED: AtomicBool = AtomicBool::new(false);

extern "C" fn request_stop(_: libc::c_int) {
    STOPPED.store(true, Ordering::Release);
}

/// 独立guardian进程的停止信号准入；来源：PF-06原资源退休合同，无Java对等对象。
/// 停止只取消公开交付；原认证出生和私有恢复仍使用原期限与原owner。
pub(super) struct LinuxGuardianShutdown;

impl LinuxGuardianShutdown {
    /// 参数：无；返回：原进程SIGTERM/SIGINT停止标记，安装失败在工作出生前拒绝。
    /// 只在新单连接、尚无其他线程的进程使用；不重新清零已收到的停止请求。
    pub(super) fn install() -> io::Result<&'static AtomicBool> {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = request_stop as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for signal in [libc::SIGTERM, libc::SIGINT] {
            if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(&STOPPED)
    }
}
