use crate::JobHandle;
use crate::native_worker::NativeWorker;
use std::path::PathBuf;
use std::sync::{Arc, Weak, atomic::AtomicBool};

/// 根目录弱订阅；仅 managed 模式强持线程记录，旧模式不积累无人回收句柄。
/// 来源：DiskGraph 原生 Rust FFI 的同根合并与关闭契约。
pub(crate) struct NativeJobEntry {
    pub(crate) root: PathBuf,
    pub(crate) subscriber: Weak<JobHandle>,
    pub(crate) worker: Option<Arc<NativeWorker>>,
    pub(crate) cancel: Arc<AtomicBool>,
}

impl NativeJobEntry {
    /// 记录真实扫描。参数：root 为规范路径，handle 为订阅者，managed 指定所有权；返回：登记。
    pub(crate) fn new(root: PathBuf, handle: &Arc<JobHandle>, managed: bool) -> Self {
        Self {
            root,
            subscriber: Arc::downgrade(handle),
            worker: managed.then(|| handle.worker.clone()),
            cancel: handle.cancel.clone(),
        }
    }
}
