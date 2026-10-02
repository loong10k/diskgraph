use crate::{PlanItem, RecoveryRule};
use diskgraph_core::{FileActionKind, PrincipalId, ScopeId};
use serde::{Deserialize, Serialize};

/// 绑定范围、主体、动作、期限与预算的不可变计划。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// An immutable plan. Nothing here mutates after creation; `state` only moves
/// forward (validated -> expired/revoked/applied).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub plan_id: String,
    pub scope_id: ScopeId,
    pub principal: PrincipalId,
    pub action: FileActionKind,
    pub items: Vec<PlanItem>,
    /// Target directory for move/copy/restore, as a locator key.
    pub target_locator_key: Option<String>,
    pub policy_version: u64,
    pub max_bytes: u64,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    /// How a failed or completed trash can be undone.
    pub recovery: RecoveryRule,
    /// Bytes the plan expects to move, for the final budget check.
    pub expected_bytes: u64,
}
