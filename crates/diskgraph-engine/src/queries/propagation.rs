//! 按关系规定影响传播方向，保护及内容身份关系不传播执行影响。

use diskgraph_core::Relation;

/// 按关系规定影响传播方向，保护及内容身份关系不传播执行影响。
/// 来源：原生 Rust diskgraph-engine::Propagation。
/// How a relation's influence propagates when asking "what does this affect?".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Propagation {
    /// Edges pointing at the target (what depends on it).
    Incoming,
    /// Edges pointing away from the target (what it depends on).
    Outgoing,
    /// Both directions, de-duplicated per (entity, relation) pair.
    Both,
}
/// 返回关系的既有传播方向白名单。
/// 参数：relation 为类型化关系。
/// 返回：允许方向或不传播的 None。
/// The direction `impact` traverses for each relation. Relations absent from
/// this table never propagate, so an undirected walk cannot happen by accident.
pub fn impact_propagation(relation: Relation) -> Option<Propagation> {
    Some(match relation {
        // Build artifacts of a project: touching the project affects its outputs.
        Relation::OwnedByProject | Relation::Contains => Propagation::Incoming,
        // What a project owns is what it rebuilds into.
        Relation::RebuildableBy | Relation::Declares => Propagation::Outgoing,
        Relation::OwnedByApplication | Relation::UsedByProcess => Propagation::Both,
        // Protection and content identity never propagate as "affected".
        Relation::ProtectedBy | Relation::SameContentAs => return None,
    })
}
