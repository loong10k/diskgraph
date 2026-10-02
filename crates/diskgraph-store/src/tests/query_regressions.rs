use crate::SqliteSnapshotStore;
use crate::history_queries::ORDERED_NODES_SQL;
use rusqlite::params;
use serde_json::to_string;

use super::fixtures::graph;
#[test]
fn bidirectional_relation_page_uses_adjacency_indexes() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let store = crate::SqliteSnapshotStore::open_in_memory().unwrap();
    store.connection.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<200000) INSERT INTO relations(snapshot_id,edge_id,source_entity_id,target_entity_id,relation,edge_json) SELECT 'fixture',printf('edge-%09d',x),'unrelated','other','contains','not-json' FROM n;").unwrap();
    let steps = Arc::new(AtomicU64::new(0));
    let observed = steps.clone();
    store
        .connection
        .progress_handler(
            100,
            Some(move || {
                observed.fetch_add(100, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let (edges, more) = store
        .edges_filtered_page("fixture", "absent", None, None, None, 1, 4096)
        .unwrap();
    assert!(edges.is_empty());
    assert!(!more);
    assert!(
        steps.load(Ordering::Relaxed) < 1000,
        "empty entity page walked unrelated relations: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}

#[test]
fn relation_pages_decode_only_requested_edges_and_keep_a_stable_cursor() {
    use diskgraph_core::{AssertionKind, Relation, RelationEdge};

    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store
        .publish_revision("job", &graph("edge-page", 10), "rev-edge-page", 1)
        .unwrap();
    for number in 1..=2 {
        let edge = RelationEdge {
            edge_id: format!("edge-{number}"),
            source_entity_id: "source".into(),
            relation: Relation::Contains,
            target_entity_id: "target".into(),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![],
        };
        store
            .connection
            .execute(
                "INSERT INTO relations VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "edge-page",
                    edge.edge_id,
                    edge.source_entity_id,
                    edge.relation.wire_name(),
                    edge.target_entity_id,
                    to_string(&edge).unwrap()
                ],
            )
            .unwrap();
    }
    // The next edge is deliberately undecodable. A one-edge page must
    // still succeed; only an attempt to read that edge may report error.
    store.connection.execute(
        "INSERT INTO relations VALUES ('edge-page', 'edge-3', 'source', 'contains', 'target', '{bad-json')",
        [],
    ).unwrap();

    let (first, more) = store
        .edges_from_page("edge-page", "source", None, 1)
        .unwrap();
    assert_eq!(
        first
            .iter()
            .map(|edge| edge.edge_id.as_str())
            .collect::<Vec<_>>(),
        vec!["edge-1"]
    );
    assert!(more);
    let (second, more) = store
        .edges_from_page("edge-page", "source", Some("edge-1"), 1)
        .unwrap();
    assert_eq!(
        second
            .iter()
            .map(|edge| edge.edge_id.as_str())
            .collect::<Vec<_>>(),
        vec!["edge-2"]
    );
    assert!(more);
    let (incoming, _) = store.edges_to_page("edge-page", "target", None, 1).unwrap();
    assert_eq!(incoming[0].edge_id, "edge-1");
    assert!(
        store
            .edges_from_page("edge-page", "source", Some("edge-2"), 1)
            .is_err()
    );
}

#[test]
fn ordered_history_cursor_uses_an_index_before_decoding_nodes() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store
        .publish_revision("job", &graph("ordered", 10), "rev-ordered", 1)
        .unwrap();
    let plans: Vec<String> = {
        let mut statement = store
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {ORDERED_NODES_SQL}"))
            .unwrap();
        statement
            .query_map(params!["ordered", "/tmp/diskgraph-test"], |row| {
                row.get::<_, String>(3)
            })
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    };
    assert!(
        plans
            .iter()
            .any(|plan| plan.contains("USING INDEX nodes_by_locator_path")),
        "{plans:?}"
    );
    assert!(
        !plans.iter().any(|plan| plan.contains("TEMP B-TREE")),
        "{plans:?}"
    );
}

#[test]
fn unknown_child_count_uses_a_sparse_index() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    let known = "COALESCE(read_error, json_extract(NULLIF(node_json, ''), '$.read_error'), 0) = 0 AND COALESCE(json_extract(NULLIF(node_json, ''), '$.size_known'), 1) = 1";
    let plans: Vec<String> = store
        .connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM nodes WHERE snapshot_id = ?1 AND parent_id = ?2 AND NOT ({known})"
        ))
        .unwrap()
        .query_map(params!["snapshot", 1], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert!(
        plans
            .iter()
            .any(|plan| plan.contains("nodes_by_unknown_parent")),
        "{plans:?}"
    );
}
