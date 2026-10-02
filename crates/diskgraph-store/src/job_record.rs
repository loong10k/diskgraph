use crate::{JobKind, JobState};
use diskgraph_core::{PrincipalId, ScopeId};
use serde::{Deserialize, Serialize};

/// 绑定真实主体、owner、lease 和 fencing 的任务记录。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// One durable job with its fencing owner token.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub job_id: String,
    pub scope_id: ScopeId,
    pub kind: JobKind,
    pub state: JobState,
    pub created_at_unix_ms: u64,
    pub heartbeat_unix_ms: u64,
    pub owner: String,
    /// 每次重新认领递增，旧 owner 的写入必须携带并验证此值。
    #[serde(default)]
    pub fencing_token: u64,
    /// 当前租约结束时间；默认 30 秒。
    #[serde(default)]
    pub lease_expires_unix_ms: u64,
    /// The principal who requested the job; per-principal quotas count on it.
    pub principal: PrincipalId,
}
