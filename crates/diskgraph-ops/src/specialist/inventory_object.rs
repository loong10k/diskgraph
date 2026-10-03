//! inventory_object：既有文件操作职责的原生 Rust 实现。
use std::path::PathBuf;

/// 专家工具列出的单个可审查对象及其原生路径、种类和字节数。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::InventoryObject`，保留既有语义。
/// One exact object a specialist adapter offers for review. `kind` says what
/// it is; nothing here is a general directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryObject {
    pub path: PathBuf,
    pub bytes: u64,
    /// e.g. `cargo-target`; the kind bounds what a plan may do with it.
    pub kind: &'static str,
}
