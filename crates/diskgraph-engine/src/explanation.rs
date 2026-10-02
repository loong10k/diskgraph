//! 保留实体、关系边与证据的既有三元组返回别名。

/// 保留实体、关系边与证据的既有三元组返回别名。
/// 来源：原生 Rust diskgraph-engine::Explanation。
/// What one explanation returns: the entity, its edges, and their evidence.
pub type Explanation = (
    diskgraph_core::Entity,
    Vec<diskgraph_core::RelationEdge>,
    Vec<diskgraph_core::EvidenceRecord>,
);
