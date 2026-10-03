//! apply_request：既有文件操作职责的原生 Rust 实现。
use crate::fault_point::FaultPoint;

/// 执行已批准计划的请求及幂等键、可选演练故障。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::ApplyRequest`，保留既有语义。
/// The request that applies a plan.
pub struct ApplyRequest<'a> {
    pub plan_id: &'a str,
    pub approval_ref: &'a str,
    /// Reusing a key with the same request returns the original operation;
    /// reusing it with a different request is refused.
    pub idempotency_key: &'a str,
    pub fault: Option<FaultPoint>,
}
