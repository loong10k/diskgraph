use crate::native_lifecycle::NativeLifecycle;
use std::sync::Arc;

/// 路径 I/O 之前取得的请求局部准入；所有返回和 unwind 都归还同一会话计数。
/// 来源：DiskGraph 原生 Rust FFI 生命周期，不创建额外执行 owner。
pub(crate) struct NativeAdmission {
    pub(crate) lifecycle: Arc<NativeLifecycle>,
}

impl Drop for NativeAdmission {
    fn drop(&mut self) {
        self.lifecycle.release_admission();
    }
}
