//! 记录到达的实体、关系与深度，不提供执行授权。

use diskgraph_core::Relation;

/// 记录到达的实体、关系与深度，不提供执行授权。
/// 来源：原生 Rust diskgraph-engine::ImpactEntry。
/// One affected entity with the relation and depth that reached it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct ImpactEntry {
    pub entity_id: String,
    pub relation: Relation,
    pub depth: usize,
}
