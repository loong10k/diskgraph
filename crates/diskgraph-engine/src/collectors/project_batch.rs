//! 保存绑定单一快照的 collector run、实体、证据和关系批次。

use diskgraph_core::{CollectorRun, Entity, EvidenceRecord, RelationEdge};

/// 保存绑定单一快照的 collector run、实体、证据和关系批次。
/// 来源：原生 Rust diskgraph-engine::ProjectBatch。
/// The collected batch for one snapshot.
pub struct ProjectBatch {
    pub run: CollectorRun,
    pub entities: Vec<Entity>,
    pub evidence: Vec<EvidenceRecord>,
    pub edges: Vec<RelationEdge>,
}
