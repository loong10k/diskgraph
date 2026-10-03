//! live_item：既有文件操作职责的原生 Rust 实现。
use std::path::PathBuf;

/// 执行前复核的源对象、恢复引用和计划固定的已批准文件版本。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::LiveItem`，保留既有语义。
/// One planned object, revalidated against the live filesystem.
pub(super) struct LiveItem {
    pub(super) path: PathBuf,
    /// The identity revalidated at apply time; kept so the operation record can
    /// name exactly which object was touched.
    pub(super) identity: Option<String>,
    /// The recovery record this item restores from, for restore plans.
    pub(super) recovery_ref: Option<String>,
    pub(super) bytes: u64,
    pub(super) approved_version: Option<std::fs::Metadata>,
}
