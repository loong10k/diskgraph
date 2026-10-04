//! 请求局部真实 TLS 析构屏障，只用于 coordinator 生命周期测试。
use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

thread_local! {
    static EXIT: RefCell<Option<NativeWorkerExitBarrier>> = const { RefCell::new(None) };
}

/// worker 主体返回之后仍阻塞真实线程退出的屏障；来源：Rust TLS 析构验收。
pub(crate) struct NativeWorkerExitBarrier {
    reached: Sender<()>,
    release: Receiver<()>,
    released: Sender<bool>,
}

impl NativeWorkerExitBarrier {
    /// 在当前真实 worker 安装单次线程局部退出屏障。
    /// 参数：reached/release/released 为阶段、释放和主动释放见证通道。
    /// 返回：无；析构设置有限救援期限，避免失败夹具永久挂起。
    pub(crate) fn install(reached: Sender<()>, release: Receiver<()>, released: Sender<bool>) {
        EXIT.with(|slot| {
            *slot.borrow_mut() = Some(Self {
                reached,
                release,
                released,
            });
        });
    }
}

impl Drop for NativeWorkerExitBarrier {
    fn drop(&mut self) {
        let _ = self.reached.send(());
        let released = self.release.recv_timeout(Duration::from_secs(60)).is_ok();
        let _ = self.released.send(released);
    }
}
