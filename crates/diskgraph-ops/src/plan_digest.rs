//! plan_digest：既有文件操作职责的原生 Rust 实现。
use diskgraph_store::Plan;
use sha2::Digest;
use sha2::Sha256;

/// 计算计划 JSON 的稳定摘要。
/// 参数：plan 为完整计划记录。
/// 返回：序列化数据的 SHA-256 十六进制字符串。
/// The canonical SHA-256 digest of one complete immutable plan. A new plan,
/// even for the same files, needs its own approval.
pub fn plan_digest(plan: &Plan) -> String {
    // Plan consists only of deterministic serde fields. Including the plan
    // ID, deadline, recovery reference and exact byte count means an old
    // approval cannot authorize a newly issued or extended plan.
    let encoded = serde_json::to_vec(plan).expect("plan serialization is infallible");
    hex::encode(Sha256::digest(encoded))
}

/// 计算幂等请求摘要。
/// 参数：request 为执行请求。
/// 返回：原有请求字段组成的摘要。
/// The digest that identifies an apply request, so a reused idempotency key
/// with a different approval is a conflict rather than a silent merge.
pub(super) fn apply_request_digest(plan: &Plan, approval_ref: &str) -> String {
    let encoded = serde_json::to_vec(&(plan_digest(plan), approval_ref))
        .expect("apply request serialization is infallible");
    hex::encode(Sha256::digest(encoded))
}
