//! revision 候选阻止项回归；来源：EV-05 / D28，无 Java 对应对象。
use crate::{SqliteSnapshotStore, tests::graph};
use diskgraph_core::{
    AssertionKind, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord, Polarity,
    QueryBudget, Relation, RelationEdge, query_deadline,
};

fn fixture(relation: Relation, node_id: u64) -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut tree = graph("snapshot", 100);
    let mut child = tree.nodes[1].clone();
    child.id = 3;
    child.parent_id = Some(2);
    child.name = "nested".into();
    child.locator =
        diskgraph_core::ResourceLocator::NativePath("/tmp/diskgraph-test/cache/nested".into());
    tree.nodes.push(child);
    let mut evidence = tree.evidence[0].clone();
    evidence.node_id = 3;
    tree.evidence.push(evidence);
    store.append_staging_nodes("job", &tree.nodes).unwrap();
    store
        .publish_revision_owned("job", &tree, "old", 1, Some(("server", "scope")))
        .unwrap();
    let batch = CollectorBatch {
        run: CollectorRun {
            run_id: "run".into(),
            snapshot_id: "snapshot".into(),
            collector_id: "fixture".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 1,
            coverage_complete: false,
            errors: vec![],
        },
        entities: vec![
            Entity {
                entity_id: "resource".into(),
                kind: EntityKind::Resource,
                identity: format!("{{\"node_id\":{node_id}}}"),
                display: "cache".into(),
                source_run_id: "run".into(),
            },
            Entity {
                entity_id: "target".into(),
                kind: if relation == Relation::ProtectedBy {
                    EntityKind::ProtectionPolicy
                } else {
                    EntityKind::Process
                },
                identity: "observed target".into(),
                display: "target".into(),
                source_run_id: "run".into(),
            },
        ],
        evidence: vec![EvidenceRecord {
            evidence_id: "evidence".into(),
            run_id: "run".into(),
            basis: "positive observation".into(),
            observed_at_unix_ms: 1,
            expires_at_unix_ms: Some(2),
            confidence: 100,
            input_fingerprint: "fixture".into(),
        }],
        edges: vec![RelationEdge {
            edge_id: "edge".into(),
            source_entity_id: "resource".into(),
            target_entity_id: "target".into(),
            relation,
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![("evidence".into(), Polarity::Supports)],
        }],
    };
    store
        .publish_collector_revision(
            "old",
            "new",
            2,
            ("server", "scope"),
            &batch,
            &[("run", "active")],
        )
        .unwrap();
    store
}

fn selected(store: &SqliteSnapshotStore, revision: &str) -> Vec<u64> {
    let budget = QueryBudget::default();
    let result = store
        .candidate_selection_for_revision_until(
            revision,
            1000,
            budget,
            query_deadline(budget).unwrap(),
        )
        .unwrap();
    assert!(result.complete);
    result.candidates.iter().map(|(node, _)| node.id).collect()
}

#[test]
fn active_typed_occupancy_blocks_candidates_only_in_its_revision() {
    let store = fixture(Relation::UsedByProcess, 2);
    assert_eq!(selected(&store, "old"), vec![2]);
    assert!(
        selected(&store, "new").is_empty(),
        "typed occupancy was ignored"
    );
    // 保留旧 store 可信接口的静态证据语义；对外调用必须指定 revision。
    assert_eq!(
        store
            .candidate_selection("snapshot", 1000, QueryBudget::default())
            .unwrap()
            .candidates
            .len(),
        1
    );
}

#[test]
fn typed_protection_excludes_both_ancestors_and_descendants() {
    for node_id in [1, 2, 3] {
        let store = fixture(Relation::ProtectedBy, node_id);
        assert!(
            selected(&store, "new").is_empty(),
            "protected node {node_id} escaped containment exclusion"
        );
    }
}

#[test]
fn dependency_only_assertions_do_not_block_current_candidates() {
    let store = fixture(Relation::UsedByProcess, 2);
    store
        .connection
        .execute(
            "INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation,locator_writer_generation,native_observation_writer_generation) VALUES ('dependency','snapshot',3,10,11,12)",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO revision_runs VALUES ('dependency','run','dependency_only')",
            [],
        )
        .unwrap();
    crate::collector_protocol::seal(&store.connection, "dependency").unwrap();
    assert_eq!(selected(&store, "dependency"), vec![2]);
}
