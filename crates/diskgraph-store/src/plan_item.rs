use serde::{Deserialize, Serialize};

/// 计划中的精确资源、身份、源版本和后代边界。
/// 来源：DiskGraph 原生 Rust 存储设计；无 Java 对应实现。
/// One concrete object inside a plan: a resolved locator plus the identity
/// observed when the plan was created.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// Stable identifier (node id) inside the bound revision.
    pub node_id: u64,
    /// Lossless locator key (raw bytes, not a display string).
    pub locator_key: String,
    /// Volume + file identity captured at plan time, re-checked before apply.
    pub identity: Option<String>,
    /// Digest of the exact file contents and high-resolution metadata, or
    /// the complete directory membership and descendant contents. Legacy
    /// plans without this field must be re-planned before a file action.
    #[serde(default)]
    pub source_fingerprint: Option<String>,
    /// True when the plan expands to descendants (directory boundary).
    pub includes_descendants: bool,
    /// The recovery record a restore plan is derived from. Absent for every
    /// action except restore.
    #[serde(default)]
    pub recovery_ref: Option<String>,
}
