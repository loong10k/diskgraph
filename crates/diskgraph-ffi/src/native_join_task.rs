use crate::native_worker::NativeWorker;
use std::sync::Arc;
use std::thread::JoinHandle;

/// manager 从登记表取出的唯一线程责任；unwind 仍真实 join 后才通知。
/// 来源：DiskGraph 原生 Rust PF-06，两级宿主回收，不创建额外线程。
pub(crate) struct NativeJoinTask {
    worker: Arc<NativeWorker>,
    thread: Option<JoinHandle<()>>,
}

impl NativeJoinTask {
    /// 接管句柄。参数：worker 为结果记录，thread 为唯一句柄；返回：执行责任。
    pub(crate) fn new(worker: Arc<NativeWorker>, thread: JoinHandle<()>) -> Self {
        Self {
            worker,
            thread: Some(thread),
        }
    }

    /// 在 manager 或非 UI 宿主线程执行真实 join。参数：无；返回：真实线程结果。
    pub(crate) fn finish(mut self) -> bool {
        self.join()
    }

    fn join(&mut self) -> bool {
        if let Some(thread) = self.thread.take() {
            let success = thread.join().is_ok();
            self.worker.complete(success);
            success
        } else {
            true
        }
    }
}

impl Drop for NativeJoinTask {
    fn drop(&mut self) {
        self.join();
    }
}
