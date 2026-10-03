//! operation_view：既有文件操作职责的原生 Rust 实现。
use diskgraph_core::ScopeId;

/// 包含操作标识、范围、终态、单项结果及完成/剩余计数的操作视图。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::OperationView`，保留既有语义。
/// One operation as an agent sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationView {
    pub operation_id: String,
    pub plan_id: String,
    pub scope_id: ScopeId,
    pub state: diskgraph_store::OperationState,
    pub items: Vec<diskgraph_store::OperationItem>,
    /// Items that already ran; a cancellation never rewrites these.
    pub completed: usize,
    pub remaining: usize,
}
