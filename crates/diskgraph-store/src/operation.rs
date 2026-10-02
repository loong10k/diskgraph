use crate::OperationState;
use diskgraph_core::{PrincipalId, ScopeId};
use serde::{Deserialize, Serialize};

/// 幂等请求对应的实际操作、主体、状态和时间记录。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// An operation as stored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub operation_id: String,
    pub plan_id: String,
    pub scope_id: ScopeId,
    pub principal: PrincipalId,
    pub idempotency_key: String,
    pub request_digest: String,
    pub state: OperationState,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}
