//! side：既有文件操作职责的原生 Rust 实现。

/// 区分路径复核时的源端与目标端。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::Side`，保留既有语义。
/// Which end of a move a path belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Source,
    Target,
}
