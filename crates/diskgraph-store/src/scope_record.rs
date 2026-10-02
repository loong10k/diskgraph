use diskgraph_core::{Locator, ScopeId};
use serde::{Deserialize, Serialize};

/// 注册范围的无损根定位、卷身份与撤销状态。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// A registered observation scope with its lossless root locator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ScopeRecord {
    pub scope_id: ScopeId,
    pub root: Locator,
    pub volume_id: Option<String>,
    pub created_at_unix_ms: u64,
    pub revoked: bool,
}
