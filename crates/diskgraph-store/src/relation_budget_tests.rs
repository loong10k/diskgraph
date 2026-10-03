use crate::tests::graph;
use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::{QueryBudget, QueryReadBudget, TruncationReason, query_deadline};
use rusqlite::params;

fn reads(max_edges: usize, max_bytes: usize) -> QueryReadBudget {
    let budget = QueryBudget {
        max_edges,
        max_response_bytes: max_bytes,
        ..QueryBudget::default()
    };
    QueryReadBudget::new(budget, query_deadline(budget).unwrap()).unwrap()
}

#[test]
fn evidence_raw_exact_limit_and_one_byte_excess_precede_deserialization() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = graph("exact", 100);
    let expected = serde_json::to_string(&graph.evidence[0]).unwrap().len();
    store.save(&graph).unwrap();
    assert_eq!(
        store
            .evidence_with_budget("exact", 2, &mut reads(1, expected))
            .unwrap(),
        graph.evidence
    );
    let mut budget = reads(1, expected - 1);
    assert!(matches!(
        store.evidence_with_budget("exact", 2, &mut budget),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(budget.stopped(), Some(TruncationReason::ByteLimit));
    store
        .connection
        .execute(
            "UPDATE evidence SET evidence_json=json_set(evidence_json,'$.confidence',300)",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.evidence_with_budget("exact", 2, &mut reads(1, 4096)),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn required_evidence_cumulative_bytes_do_not_return_a_partial_node() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("sum", 100);
    graph.evidence.push(graph.evidence[0].clone());
    let bytes = serde_json::to_string(&graph.evidence[0]).unwrap().len();
    store.save(&graph).unwrap();
    let mut budget = reads(2, bytes * 2 - 1);
    assert!(matches!(
        store.evidence_with_budget("sum", 2, &mut budget),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(budget.stopped(), Some(TruncationReason::ByteLimit));
    assert_eq!(budget.remaining_edges(), 1);
    assert_eq!(
        store
            .evidence_with_budget("sum", 2, &mut reads(2, bytes * 2))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn selected_node_raw_fields_are_gated_before_bad_locator_parse() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("node", 100)).unwrap();
    store
        .connection
        .execute(
            "UPDATE nodes SET name=?1,locator_key='123' WHERE id=2",
            ["x".repeat(100_000)],
        )
        .unwrap();
    let mut budget = reads(2, 65536);
    assert!(matches!(
        store.node_with_budget("node", 2, &mut budget),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(budget.stopped(), Some(TruncationReason::ByteLimit));
    store
        .connection
        .execute("UPDATE nodes SET name='small' WHERE id=2", [])
        .unwrap();
    assert!(matches!(
        store.node_with_budget("node", 2, &mut reads(2, 65536)),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn bad_huge_lookahead_is_only_an_existence_probe() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    let edge = serde_json::json!({"edge_id":"a","source_entity_id":"source","target_entity_id":"target","relation":"protected_by","assertion_kind":"observed","evidence_refs":[]}).to_string();
    store
        .connection
        .execute(
            "INSERT INTO relations VALUES ('snapshot','a','source','protected_by','target',?1)",
            [edge],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO relations VALUES ('snapshot','b','source','protected_by','target',?1)",
            [vec![255u8; 100_000]],
        )
        .unwrap();
    let mut budget = reads(1, 4096);
    let (page, more) = store
        .edges_with_budget_page("snapshot", "source", Some(true), None, None, 1, &mut budget)
        .unwrap();
    assert_eq!(page.len(), 1);
    assert!(more);
    assert_eq!(budget.remaining_edges(), 0);
    let (page, more) = store
        .edges_with_budget_page(
            "snapshot",
            "source",
            Some(true),
            None,
            Some("a"),
            0,
            &mut budget,
        )
        .unwrap();
    assert!(page.is_empty());
    assert!(more);
}

#[test]
fn malformed_budgeted_edge_is_not_hidden_as_a_truncation() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.connection.execute("INSERT INTO relations VALUES ('snapshot','a','source','protected_by','target','bad JSON')", []).unwrap();
    assert!(matches!(
        store.edges_with_budget_page(
            "snapshot",
            "source",
            Some(true),
            None,
            None,
            1,
            &mut reads(1, 4096)
        ),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn self_loop_decoded_in_both_directions_uses_two_units() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    let edge = serde_json::json!({"edge_id":"a","source_entity_id":"same","target_entity_id":"same","relation":"used_by_process","assertion_kind":"observed","evidence_refs":[]}).to_string();
    store
        .connection
        .execute(
            "INSERT INTO relations VALUES ('snapshot','a','same','used_by_process','same',?1)",
            [edge],
        )
        .unwrap();
    let mut budget = reads(2, 4096);
    for outgoing in [false, true] {
        assert_eq!(
            store
                .edges_with_budget_page(
                    "snapshot",
                    "same",
                    Some(outgoing),
                    None,
                    None,
                    1,
                    &mut budget
                )
                .unwrap()
                .0
                .len(),
            1
        );
    }
    assert_eq!(budget.remaining_edges(), 0);
}

#[test]
fn candidate_late_node_is_atomic_and_gap_stays_exact() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("prefix", 1000);
    let mut sibling = graph.nodes[1].clone();
    sibling.id = 3;
    sibling.name = "small".into();
    sibling.subtree_bytes = 300;
    sibling.locator =
        diskgraph_core::ResourceLocator::NativePath("/tmp/diskgraph-test/small".into());
    graph.nodes.push(sibling);
    let mut evidence = graph.evidence[0].clone();
    evidence.node_id = 3;
    graph.evidence.push(evidence);
    store.save(&graph).unwrap();
    let result = store
        .candidate_selection(
            "prefix",
            2000,
            QueryBudget {
                max_edges: 1,
                ..QueryBudget::default()
            },
        )
        .unwrap();
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].0.id, 2);
    assert_eq!(
        (result.selected_bytes, result.remaining_bytes),
        (1000, 1000)
    );
    assert_eq!(result.truncated, Some(TruncationReason::EdgeLimit));
    assert!(!result.complete);
}

#[test]
fn legacy_unknown_and_unicode_node_behavior_is_preserved() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("legacy", 100);
    graph.nodes[1].size_known = false;
    graph.nodes[1].name = "中文🙂".into();
    store.save(&graph).unwrap();
    let mut budget = reads(2, 65536);
    let node = store
        .node_with_budget("legacy", 2, &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(node, graph.nodes[1]);
    assert!(
        store
            .candidate_selection("legacy", 1, QueryBudget::default())
            .unwrap()
            .candidates
            .is_empty()
    );
    store
        .connection
        .execute(
            "UPDATE nodes SET read_error=NULL,file_volume_id=NULL,file_id=NULL WHERE id=1",
            [],
        )
        .unwrap();
    assert!(
        store
            .node_with_budget("legacy", 1, &mut reads(2, 65536))
            .unwrap()
            .is_some()
    );
    store
        .connection
        .execute(
            "UPDATE nodes SET kind=NULL,node_json=?1 WHERE id=1",
            params![serde_json::to_string(&graph.nodes[0]).unwrap()],
        )
        .unwrap();
    assert_eq!(
        store
            .node_with_budget("legacy", 1, &mut reads(2, 65536))
            .unwrap(),
        Some(graph.nodes[0].clone())
    );
}

#[test]
fn legacy_zero_raw_cap_keeps_empty_and_format_error_semantics() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    assert_eq!(
        store
            .edges_filtered_page("snapshot", "source", Some(true), None, None, 1, 0)
            .unwrap(),
        (Vec::new(), false)
    );
    store
        .connection
        .execute(
            "INSERT INTO relations VALUES ('snapshot','a','source','protected_by','target','x')",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.edges_filtered_page("snapshot", "source", Some(true), None, None, 1, 0),
        Err(StoreError::BudgetExceeded)
    ));
    store
        .connection
        .execute("UPDATE relations SET edge_json=''", [])
        .unwrap();
    assert!(matches!(
        store.edges_filtered_page("snapshot", "source", Some(true), None, None, 1, 0),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn bounded_candidate_evidence_preserves_the_legacy_text_column_contract() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = graph("blob", 100);
    store.save(&graph).unwrap();
    let json = serde_json::to_vec(&graph.evidence[0]).unwrap();
    store
        .connection
        .execute("UPDATE evidence SET evidence_json=?1", [json])
        .unwrap();
    assert!(matches!(
        store.evidence("blob", 2),
        Err(StoreError::Sqlite(_))
    ));
    assert!(
        matches!(
            store.evidence_with_budget("blob", 2, &mut reads(1, 4096)),
            Err(StoreError::Sqlite(_))
        ),
        "bounded evidence accepted a BLOB that the old TEXT reader refuses"
    );
}

#[test]
fn legacy_relation_page_preserves_text_type_without_decoding_its_lookahead() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    let edge = serde_json::json!({"edge_id":"a","source_entity_id":"source","target_entity_id":"target","relation":"protected_by","assertion_kind":"observed","evidence_refs":[]});
    store
        .connection
        .execute(
            "INSERT INTO relations VALUES ('snapshot','a','source','protected_by','target',?1)",
            [serde_json::to_vec(&edge).unwrap()],
        )
        .unwrap();
    assert!(
        matches!(
            store.edges_from_page("snapshot", "source", None, 1),
            Err(StoreError::Sqlite(_))
        ),
        "legacy relation page accepted a BLOB column as TEXT"
    );
}
