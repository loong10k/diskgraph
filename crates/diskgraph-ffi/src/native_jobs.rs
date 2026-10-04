use crate::native_job_entry::NativeJobEntry;

/// 单会话准入与线程登记，manager 错误独立于业务完成事实。
/// 来源：DiskGraph 原生 Rust PF-06 生命周期状态。
#[derive(Default)]
pub(crate) struct NativeJobs {
    pub(crate) in_flight: usize,
    pub(crate) entries: Vec<NativeJobEntry>,
    pub(crate) failure: Option<&'static str>,
}
