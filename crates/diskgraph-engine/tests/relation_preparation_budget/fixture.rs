//! 已发布合法大目标的关系查询夹具；来源：原生 Rust Q-08。
use diskgraph_core::{
    AssertionKind, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord, Polarity,
    Relation, RelationEdge, ScopeId,
};
use diskgraph_store::SqliteSnapshotStore;

#[path = "../history_preparation_budget/fixture.rs"]
mod base_fixture;

/// 真实注册授权与公开持久批次，不把合成导入声称为原生采样。
/// 来源：DiskGraph 原生 Rust Q-08 准备成本验收。
pub(crate) struct Fixture {
    pub(crate) base: base_fixture::Fixture,
    pub(crate) scope: ScopeId,
}
impl Fixture {
    /// 参数：bytes 为合法 snapshot ID 长度；返回：固定短 revision 的授权夹具。
    pub(crate) fn new(bytes: usize) -> Self {
        Self::with_edge(bytes, 0)
    }

    /// 参数：bytes 为头长度，target_bytes 为真实有向关系的端点长度；返回：固定合法批次。
    pub(crate) fn with_edge(bytes: usize, target_bytes: usize) -> Self {
        let base = base_fixture::Fixture::new(8, bytes);
        let mut store =
            SqliteSnapshotStore::open(&base.engine.data_dir().join("diskgraph.sqlite")).unwrap();
        let owner = store.revision_ownership("right").unwrap().unwrap();
        let scope = ScopeId::new(owner.1.clone()).unwrap();
        let snapshot = store.revision("right").unwrap().snapshot_id;
        let run = CollectorRun {
            run_id: "preparation-run".into(),
            snapshot_id: snapshot,
            collector_id: "preparation-fixture".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 3,
            coverage_complete: true,
            errors: vec![],
        };
        let entity = Entity {
            entity_id: "entity".into(),
            kind: EntityKind::Resource,
            identity: "fixture".into(),
            display: "fixture".into(),
            source_run_id: run.run_id.clone(),
        };
        let mut entities = vec![entity];
        let mut edges = Vec::new();
        let mut evidence = Vec::new();
        if target_bytes > 0 {
            let target = format!("target-{}", "x".repeat(target_bytes));
            entities.push(Entity {
                entity_id: target.clone(),
                kind: EntityKind::Resource,
                identity: "target-fixture".into(),
                display: "target".into(),
                source_run_id: run.run_id.clone(),
            });
            edges.push(RelationEdge {
                edge_id: "edge".into(),
                source_entity_id: target,
                target_entity_id: "entity".into(),
                relation: Relation::Contains,
                assertion_kind: AssertionKind::Observed,
                evidence_refs: vec![("fixture-evidence".into(), Polarity::Supports)],
            });
            evidence.push(EvidenceRecord {
                evidence_id: "fixture-evidence".into(),
                run_id: run.run_id.clone(),
                basis: "synthetic imported relationship".into(),
                observed_at_unix_ms: run.observed_at_unix_ms,
                expires_at_unix_ms: None,
                confidence: 100,
                input_fingerprint: "synthetic-import".into(),
            });
        }
        store
            .publish_collector_revision(
                "right",
                "selected",
                3,
                (&owner.0, &owner.1),
                &CollectorBatch {
                    run: run.clone(),
                    entities,
                    evidence,
                    edges,
                },
                &[(&run.run_id, "active")],
            )
            .unwrap();
        Self { base, scope }
    }

    /// 返回本夹具的数据位置；参数：无；返回：隔离临时目录中的路径。
    pub(crate) fn data_dir(&self) -> std::path::PathBuf {
        self.base.engine.data_dir().to_path_buf()
    }
}
