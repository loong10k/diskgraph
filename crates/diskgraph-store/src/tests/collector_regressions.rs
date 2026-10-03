use crate::SqliteSnapshotStore;

use super::fixtures::graph;
#[test]
fn collector_batches_store_validate_and_explain() {
    use diskgraph_core::{
        AssertionKind, CollectorRun, Entity, EntityKind, EvidenceRecord, Polarity, Relation,
        RelationEdge,
    };
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = graph("snap", 100);
    // publish_revision creates the snapshot row the batch attaches to.

    let run = CollectorRun {
        run_id: "run-1".into(),
        snapshot_id: "snap".into(),
        collector_id: "cargo-node-projects".into(),
        collector_version: 1,
        rule_version: 1,
        observed_at_unix_ms: 10,
        coverage_complete: true,
        errors: vec![],
    };
    let entities = vec![
        Entity {
            entity_id: "ent-res".into(),
            kind: EntityKind::Resource,
            identity: r#"{"node_id":2}"#.into(),
            display: "/tmp/diskgraph-test/cache".into(),
            source_run_id: "run-1".into(),
        },
        Entity {
            entity_id: "ent-project".into(),
            kind: EntityKind::Project,
            identity: r#"{"manifest":"Cargo.toml"}"#.into(),
            display: "diskgraph-test".into(),
            source_run_id: "run-1".into(),
        },
        Entity {
            entity_id: "ent-recipe".into(),
            kind: EntityKind::BuildRecipe,
            identity: r#"{"tool":"cargo"}"#.into(),
            display: "cargo build".into(),
            source_run_id: "run-1".into(),
        },
    ];
    let evidence = vec![EvidenceRecord {
        evidence_id: "ev-1".into(),
        run_id: "run-1".into(),
        basis: "target/ sits next to Cargo.toml in the same directory".into(),
        observed_at_unix_ms: 10,
        expires_at_unix_ms: None,
        confidence: 90,
        input_fingerprint: "fp-1".into(),
    }];
    let edges = vec![
        RelationEdge {
            edge_id: "edge-1".into(),
            source_entity_id: "ent-res".into(),
            relation: Relation::OwnedByProject,
            target_entity_id: "ent-project".into(),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![("ev-1".into(), Polarity::Supports)],
        },
        RelationEdge {
            edge_id: "edge-2".into(),
            source_entity_id: "ent-res".into(),
            relation: Relation::RebuildableBy,
            target_entity_id: "ent-recipe".into(),
            assertion_kind: AssertionKind::Derived,
            evidence_refs: vec![("ev-1".into(), Polarity::Supports)],
        },
    ];
    let batch = diskgraph_core::CollectorBatch {
        run: run.clone(),
        entities: entities.clone(),
        evidence: evidence.clone(),
        edges: edges.clone(),
    };
    store
        .publish_revision_owned_with_batch("job", &graph, "rev-x", 1, None, Some(&batch))
        .unwrap();
    // 兼容入口只能幂等重放封存后的已有选择。
    store
        .bind_runs_to_revision("rev-x", &[("run-1", "active")])
        .unwrap();

    assert_eq!(
        store.edges_to("snap", "ent-project", None).unwrap().len(),
        1
    );
    assert_eq!(
        store
            .edges_from("snap", "ent-res", Some(Relation::RebuildableBy))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .edges_from("snap", "ent-res", Some(Relation::ProtectedBy))
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store.entity("snap", "ent-project").unwrap().unwrap().kind,
        EntityKind::Project
    );
    let provenance = store
        .evidence_for_edges("snap", &["edge-1".into(), "edge-2".into()])
        .unwrap();
    assert_eq!(provenance.len(), 1);
    assert_eq!(provenance[0].evidence_id, "ev-1");
    // Cross-batch evidence references are refused.
    let dangling = vec![RelationEdge {
        edge_id: "edge-3".into(),
        source_entity_id: "ent-res".into(),
        relation: Relation::OwnedByProject,
        target_entity_id: "ent-project".into(),
        assertion_kind: AssertionKind::Observed,
        evidence_refs: vec![("ev-missing".into(), Polarity::Supports)],
    }];
    assert!(
        store
            .record_collector_batch("snap", &run, &entities, &evidence, &dangling)
            .is_err()
    );
}

#[test]
fn endpoint_violations_reject_the_whole_batch() {
    use diskgraph_core::{AssertionKind, CollectorRun, Entity, EntityKind, Relation, RelationEdge};
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("snap", 100)).unwrap();
    let run = CollectorRun {
        run_id: "run-1".into(),
        snapshot_id: "snap".into(),
        collector_id: "bad".into(),
        collector_version: 1,
        rule_version: 1,
        observed_at_unix_ms: 1,
        coverage_complete: true,
        errors: vec![],
    };
    let entities = vec![
        Entity {
            entity_id: "ent-res".into(),
            kind: EntityKind::Resource,
            identity: "{}".into(),
            display: "r".into(),
            source_run_id: "run-1".into(),
        },
        Entity {
            entity_id: "ent-proc".into(),
            kind: EntityKind::Process,
            identity: "{}".into(),
            display: "p".into(),
            source_run_id: "run-1".into(),
        },
    ];
    let illegal = vec![RelationEdge {
        edge_id: "bad-edge".into(),
        source_entity_id: "ent-res".into(),
        relation: Relation::OwnedByProject,
        target_entity_id: "ent-proc".into(),
        assertion_kind: AssertionKind::Observed,
        evidence_refs: vec![],
    }];
    assert!(
        store
            .record_collector_batch("snap", &run, &entities, &[], &illegal)
            .is_err()
    );
    // Nothing from the rejected batch may be visible.
    assert!(
        store
            .edges_from("snap", "ent-res", None)
            .unwrap()
            .is_empty()
    );
}
