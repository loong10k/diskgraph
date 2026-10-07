//! Unix HTTP 宿主的正常终止请求；信号处理器不执行恢复、锁或数据库操作。
use std::sync::atomic::{AtomicBool, Ordering};
static REQUESTED: AtomicBool = AtomicBool::new(false);
extern "C" fn request_stop(_: libc::c_int) {
    REQUESTED.store(true, Ordering::Relaxed);
}
/// 保留原 SIGTERM/SIGINT 配置的唯一进程入口守卫；来源：原生 Rust MCP 服务生命周期。
/// 仅二进制启动时安装；恢复责任仍由原 runtime/runner/Engine owner 持有。
pub(crate) struct TerminationSignal {
    old_term: libc::sigaction,
    old_int: libc::sigaction,
}
impl TerminationSignal {
    /// 参数：无，仅单个二进制入口调用；返回：原信号配置守卫或注册错误。
    pub(crate) fn install() -> std::io::Result<Self> {
        REQUESTED.store(false, Ordering::Relaxed);
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = request_stop as *const () as usize;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        let mut old_term = std::mem::MaybeUninit::uninit();
        let mut old_int = std::mem::MaybeUninit::uninit();
        if unsafe { libc::sigaction(libc::SIGTERM, &action, old_term.as_mut_ptr()) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let old_term = unsafe { old_term.assume_init() };
        if unsafe { libc::sigaction(libc::SIGINT, &action, old_int.as_mut_ptr()) } < 0 {
            let error = std::io::Error::last_os_error();
            unsafe {
                libc::sigaction(libc::SIGTERM, &old_term, std::ptr::null_mut());
            }
            return Err(error);
        }
        Ok(Self {
            old_term,
            old_int: unsafe { old_int.assume_init() },
        })
    }
    /// 参数：无；返回：是否收到正常终止请求，不代表任何原资源已回收。
    pub(crate) fn requested(&self) -> bool {
        REQUESTED.load(Ordering::Relaxed)
    }
}
impl Drop for TerminationSignal {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGTERM, &self.old_term, std::ptr::null_mut());
            libc::sigaction(libc::SIGINT, &self.old_int, std::ptr::null_mut());
        }
    }
}
