use diskgraph_core::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, NodeKind, QueryBudget,
    ResourceLocator, ScanCoverage, ScanSettings, TruncationReason,
};
use diskgraph_store::SqliteSnapshotStore;

fn node(id: u64, parent_id: Option<u64>, name: &str, bytes: u64, kind: NodeKind) -> DiskNode {
    let path = match id {
        1 => "/fixture".to_owned(),
        3 => "/fixture/blocked-dir/protected-file".to_owned(),
        _ => format!("/fixture/{name}"),
    };
    DiskNode {
        id,
        parent_id,
        locator: ResourceLocator::NativePath(path),
        name: name.into(),
        kind,
        subtree_bytes: bytes,
        direct_bytes: if kind == NodeKind::File { bytes } else { 0 },
        size_known: true,
        files: 1,
        directories: usize::from(kind == NodeKind::Directory) as u64,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    }
}

fn evidence(node_id: u64, relation: EvidenceRelation) -> EvidenceEdge {
    EvidenceEdge {
        node_id,
        relation,
        subject: "fixture".into(),
        source: "test".into(),
        observed_at_unix_ms: 1,
        confidence: 100,
    }
}

fn graph() -> DiskGraph {
    DiskGraph {
        snapshot: DiskSnapshot {
            id: "candidate-snapshot".into(),
            root: ResourceLocator::NativePath("/fixture".into()),
            volume_id: Some("fixture-volume".into()),
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: true,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![
            node(1, None, "root", 1_000, NodeKind::Directory),
            node(2, Some(1), "blocked-dir", 700, NodeKind::Directory),
            node(3, Some(2), "protected-file", 50, NodeKind::File),
            node(4, Some(1), "eligible-dir", 300, NodeKind::Directory),
            node(5, Some(1), "unrelated-file", 20, NodeKind::File),
        ],
        evidence: vec![
            evidence(2, EvidenceRelation::Rebuildable),
            evidence(3, EvidenceRelation::Protected),
            evidence(4, EvidenceRelation::Rebuildable),
        ],
    }
}

#[test]
fn positive_target_candidates_skip_unrelated_corrupt_nodes_and_report_the_gap() {
    let fixture = tempfile::tempdir().unwrap();
    let database = fixture.path().join("graph.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    store.save(&graph()).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE nodes SET name = X'ff' WHERE snapshot_id = 'candidate-snapshot' AND id = 5",
            [],
        )
        .unwrap();
    assert!(store.load("candidate-snapshot").is_err());

    let result = store
        .candidate_selection("candidate-snapshot", 500, QueryBudget::default())
        .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].0.id, 4);
    assert_eq!(result.selected_bytes, 300);
    assert_eq!(result.remaining_bytes, 200);
    assert!(result.complete);
    assert_eq!(result.truncated, None);
}

#[test]
fn candidate_result_never_calls_a_budget_cut_complete() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph();
    graph
        .evidence
        .retain(|edge| edge.relation != EvidenceRelation::Protected);
    store.save(&graph).unwrap();
    let budget = QueryBudget {
        max_nodes: 1,
        ..QueryBudget::default()
    };
    let result = store
        .candidate_selection("candidate-snapshot", 900, budget)
        .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert!(!result.complete);
    assert_eq!(result.truncated, Some(TruncationReason::NodeLimit));
    assert!(result.remaining_bytes > 0);
}

#[test]
fn migrated_prestructured_directory_remains_a_candidate() {
    let fixture = tempfile::tempdir().unwrap();
    let database = fixture.path().join("legacy.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    let graph = graph();
    store.save(&graph).unwrap();
    let legacy = serde_json::to_string(&graph.nodes[3]).unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE nodes SET kind = NULL, node_json = ?1 WHERE snapshot_id = 'candidate-snapshot' AND id = 4",
            [legacy],
        )
        .unwrap();
    let result = store
        .candidate_selection("candidate-snapshot", 1, QueryBudget::default())
        .unwrap();
    assert_eq!(result.candidates[0].0.id, 4);
}
