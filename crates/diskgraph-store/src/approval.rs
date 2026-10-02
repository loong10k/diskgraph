use diskgraph_core::{FileActionKind, PrincipalId};
use serde::{Deserialize, Serialize};

/// 绑定精确计划摘要、主体、动作和期限的可信批准。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// A trusted approval for exactly one plan digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    pub approval_ref: String,
    pub plan_id: String,
    pub plan_digest: String,
    pub principal: PrincipalId,
    pub action: FileActionKind,
    pub issued_by: String,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub revoked: bool,
}
