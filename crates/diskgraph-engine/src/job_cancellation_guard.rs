use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// 一次执行代次的取消句柄清理；来源：原生 Rust Engine 任务生命周期。
/// 只借用 Engine 已有 owner，按 Arc 身份释放，不删除后来认领代次的标志。
pub(super) struct JobCancellationGuard<'a> {
    pub(super) entries: &'a Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(super) job_id: String,
    pub(super) flag: Arc<AtomicBool>,
}

impl Drop for JobCancellationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.entries.lock()
            && entries
                .get(&self.job_id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.flag))
        {
            entries.remove(&self.job_id);
        }
    }
}
