use crate::RecoveryState;
use diskgraph_core::ScopeId;
use serde::{Deserialize, Serialize};

/// 恢复对象的原定位、回收定位、身份及消费状态。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// A durable record of where a quarantined object went and how to get it back.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub recovery_ref: String,
    pub operation_id: String,
    pub scope_id: ScopeId,
    pub original_locator: String,
    pub quarantine_locator: String,
    pub identity: String,
    pub created_at_unix_ms: u64,
    pub state: RecoveryState,
}
