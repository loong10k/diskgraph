//! 单次采集的不可分割写入载荷。

use crate::{CollectorRun, Entity, EvidenceRecord, RelationEdge};
use serde::{Deserialize, Serialize};

/// 同一运行产生的实体、证据与关系，在发布事务中整体校验和持久化。
/// 来源：DiskGraph 原生 Rust EV-05 设计；无 Java 对应对象。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CollectorBatch {
    pub run: CollectorRun,
    pub entities: Vec<Entity>,
    pub evidence: Vec<EvidenceRecord>,
    pub edges: Vec<RelationEdge>,
}
