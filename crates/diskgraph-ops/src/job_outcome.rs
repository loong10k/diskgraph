//! job_outcome：既有文件操作职责的原生 Rust 实现。

/// 操作后的范围刷新结果，包含真实任务编号和持久化任务状态。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::JobOutcome`，保留既有语义。
/// The result of the post-operation refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobOutcome {
    pub job_id: String,
    pub state: diskgraph_store::JobState,
}
