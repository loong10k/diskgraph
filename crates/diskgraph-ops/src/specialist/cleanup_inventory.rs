//! cleanup_inventory：既有文件操作职责的原生 Rust 实现。
use crate::specialist::inventory_object::InventoryObject;

/// 专家工具的审查清单、边界说明与活动构建观察，不构成批准。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::CleanupInventory`，保留既有语义。
/// What a specialist adapter observed, ready for a human to review. It is a
/// review queue, never a plan and never an authorization.
#[derive(Clone, Debug)]
pub struct CleanupInventory {
    pub adapter: &'static str,
    pub objects: Vec<InventoryObject>,
    /// Measurements the caller must see: shared directories, active builds,
    /// anything that bounds what a cleanup may honestly claim.
    pub notes: Vec<String>,
    /// True when the tool's own markers say a build is in flight; cleanup
    /// must wait.
    pub active_build: bool,
}
