//! step_result：既有文件操作职责的原生 Rust 实现。

/// 单项操作的日志种类、结果说明、处理字节与恢复引用。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::StepResult`，保留既有语义。
pub(super) struct StepResult {
    pub(super) kind: diskgraph_store::OperationItemResult,
    pub(super) detail: String,
    pub(super) bytes: u64,
    pub(super) recovery_ref: Option<String>,
}
