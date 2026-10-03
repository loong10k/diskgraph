//! fault_point：既有文件操作职责的原生 Rust 实现。

/// 恢复演练中暂停执行的故障注入位置；生产请求使用 None。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::FaultPoint`，保留既有语义。
/// Where a run may be interrupted, so the recovery contract can be tested
/// rather than assumed (P5 task 6.13). Production passes `None`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    /// After the intent is persisted, before the file changes.
    AfterIntent,
    /// After the file changed, before the result is recorded.
    AfterFileChange,
    /// A cross-volume move published the verified copy but has not yet removed
    /// the source. Drilling this exact seam proves the contract that the copy
    /// may be trusted and the source kept, never the other way round (OP-05).
    AfterCopyBeforeSourceRemoval,
}
