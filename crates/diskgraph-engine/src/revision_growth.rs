//! 保存可比较历史中同一路径的前后节点及有符号大小变化。

/// 保存可比较历史中同一路径的前后节点及有符号大小变化。
/// 来源：原生 Rust diskgraph-engine::RevisionGrowth。
/// The size change of one path between two published revisions, owned/// rather than borrowed: the answer costs two rows, so there is no reason to
/// hold a whole graph alive to describe them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionGrowth {
    pub before: diskgraph_core::DiskNode,
    pub after: diskgraph_core::DiskNode,
    pub delta_bytes: i128,
}
