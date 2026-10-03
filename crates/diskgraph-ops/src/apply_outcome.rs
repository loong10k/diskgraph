//! apply_outcome：既有文件操作职责的原生 Rust 实现。

/// 执行或幂等复用后的操作标识、状态和处理计数。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::ApplyOutcome`，保留既有语义。
/// What an apply produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyOutcome {
    pub operation_id: String,
    /// False when an existing operation was returned for a reused key.
    pub started: bool,
    pub state: diskgraph_store::OperationState,
    pub moved: usize,
    pub failed: usize,
    pub bytes: u64,
}
