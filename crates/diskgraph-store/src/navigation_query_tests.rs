//! 必要投影、旧记录和真实 SQLite 工作量；来源：OpenSpec Q-08 / 13.6。

use crate::{SqliteSnapshotStore, StoreError, tests::graph};
use diskgraph_core::{QueryBudget, QueryReadBudget, TruncationReason};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn budget(nodes: usize, bytes: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_nodes: nodes,
            max_response_bytes: bytes,
            ..QueryBudget::default()
        },
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap()
}

#[test]
fn navigation_sql_excludes_unused_columns_and_counts_required_input() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("navigation", 30);
    for node in &mut graph.nodes {
        node.reclaim_hint = Some("x".repeat(2 << 20));
    }
    store.save(&graph).unwrap();
    store
        .connection
        .authorizer(Some(|context: AuthContext<'_>| {
            if let AuthAction::Read {
                table_name: "nodes",
                column_name,
            } = context.action
                && matches!(
                    column_name,
                    "locator_key"
                        | "reclaim_hint"
                        | "file_volume_id"
                        | "file_id"
                        | "modified_unix_seconds"
                        | "direct_bytes"
                )
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    // 正控制：旧完整节点 SQL 在真实 authorizer 下失败，不能以空 hook 充数。
    assert!(store.node("navigation", 1).is_err());
    let mut reads = budget(2, 1024);
    let before = reads.remaining_raw_bytes();
    let root = store
        .navigation_node_with_budget("navigation", 1, &mut reads)
        .unwrap()
        .unwrap();
    let (children, more) = store
        .navigation_children_with_budget("navigation", 1, 0, 1, &mut reads)
        .unwrap();
    assert_eq!(root.name, "diskgraph-test");
    assert_eq!(children[0].name, "cache");
    assert!(!more);
    assert_eq!(reads.nodes_read(), 2);
    let raw = before - reads.remaining_raw_bytes();
    assert!(raw > 0 && raw < 1024, "required input={raw}");
    assert!(reads.stopped().is_none());
    eprintln!(
        "navigation required raw bytes={raw}, nodes={}",
        reads.nodes_read()
    );
}

#[test]
fn legacy_projection_preserves_unknown_and_checks_payload_id() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("legacy-navigation", 0);
    graph.nodes[1].size_known = false;
    graph.nodes[1].category_hint = Some("旧目录".into());
    graph.nodes[1].read_error = true;
    store.save(&graph).unwrap();
    let node = store
        .navigation_node_with_budget("legacy-navigation", 2, &mut budget(1, 4096))
        .unwrap()
        .unwrap();
    assert!(!node.size_known);
    assert!(node.read_error);
    assert_eq!(node.subtree_bytes, 0);
    assert_eq!(node.category_hint.as_deref(), Some("旧目录"));
    store
        .connection
        .execute(
            "UPDATE nodes SET node_json=json_set(node_json,'$.id',99) WHERE id=2",
            [],
        )
        .unwrap();
    let error = store
        .navigation_node_with_budget("legacy-navigation", 2, &mut budget(1, 4096))
        .unwrap_err();
    assert!(matches!(error, StoreError::InvalidGraph(message) if message.contains("payload ID")));
}

#[test]
fn navigation_shared_ledger_rejects_large_legacy_input_before_decoding() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("large-legacy", 1);
    graph.nodes[1].size_known = false;
    graph.nodes[1].reclaim_hint = Some("x".repeat(2 << 20));
    store.save(&graph).unwrap();
    let mut reads = budget(2, 1024);
    store
        .navigation_node_with_budget("large-legacy", 1, &mut reads)
        .unwrap();
    let error = store
        .navigation_children_with_budget("large-legacy", 1, 0, 1, &mut reads)
        .unwrap_err();
    assert!(matches!(error, StoreError::BudgetExceeded));
    assert_eq!(reads.stopped(), Some(TruncationReason::ByteLimit));
    assert_eq!(reads.nodes_read(), 1);
}

#[test]
fn navigation_page_does_not_decode_bad_continuation_and_preserves_offset() {
    let store = crate::child_aggregate_tests::wide_store(false);
    store
        .connection
        .execute("UPDATE nodes SET kind='broken' WHERE id=200001", [])
        .unwrap();
    let (items, more) = store
        .navigation_children_with_budget("wide", 1, 0, 1, &mut budget(1, 4096))
        .unwrap();
    assert_eq!(items[0].id, 200002);
    assert!(more);
    let error = store
        .navigation_children_with_budget("wide", 1, 1, 1, &mut budget(1, 4096))
        .unwrap_err();
    assert!(
        matches!(error, StoreError::InvalidGraph(message) if message.contains("unknown node kind"))
    );
}

#[test]
fn navigation_query_work_scales_with_the_page() {
    let store = crate::child_aggregate_tests::wide_store(false);
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = steps.clone();
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    for limit in [1, 5] {
        steps.store(0, Ordering::Relaxed);
        let mut reads = budget(limit, 4096);
        let (items, more) = store
            .navigation_children_with_budget("wide", 1, 0, limit as u64, &mut reads)
            .unwrap();
        let actual = steps.load(Ordering::Relaxed);
        assert_eq!(items.len(), limit);
        assert!(more);
        assert_eq!(reads.nodes_read(), limit);
        assert!(actual < 1000, "navigation walked 200k siblings: {actual}");
        eprintln!(
            "navigation page={limit}, exact VM steps={actual}, nodes={}",
            reads.nodes_read()
        );
    }
}

#[test]
fn expired_navigation_starts_no_subsequent_sql() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    store
        .connection
        .authorizer(Some(move |_: AuthContext<'_>| {
            observed.fetch_add(1, Ordering::Relaxed);
            Authorization::Deny
        }))
        .unwrap();
    let mut reads = QueryReadBudget::new(QueryBudget::default(), Instant::now()).unwrap();
    assert!(matches!(
        store.navigation_node_with_budget("anything", 1, &mut reads),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        store.navigation_children_with_budget("anything", 1, 0, 1, &mut reads),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(attempts.load(Ordering::Relaxed), 0);
}

#[test]
fn navigation_existence_probe_never_selects_large_invalid_legacy_payload() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = graph("probe-navigation", 30);
    let mut continuation = graph.nodes[1].clone();
    continuation.id = 3;
    continuation.name = "continuation".into();
    continuation.locator =
        diskgraph_core::ResourceLocator::NativePath("/tmp/diskgraph-test/continuation".into());
    continuation.subtree_bytes = 20;
    continuation.direct_bytes = 20;
    continuation.size_known = false;
    graph.nodes.push(continuation);
    store.save(&graph).unwrap();
    // JSON 文法合法，计数索引仍有效；所需页之外是巨大且字段类型错误的旧 payload。
    let payload = serde_json::json!({"size_known": false, "read_error": false, "id": "bad", "reclaim_hint": "x".repeat(2 << 20)}).to_string();
    store
        .connection
        .execute("UPDATE nodes SET node_json=?1 WHERE id=3", [&payload])
        .unwrap();
    let payload_reads = Arc::new(AtomicUsize::new(0));
    let observed = payload_reads.clone();
    store
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "nodes",
                    column_name: "node_json"
                }
            ) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Authorization::Allow
        }))
        .unwrap();
    let mut reads = budget(1, 1024);
    let (items, more) = store
        .navigation_children_with_budget("probe-navigation", 1, 0, 1, &mut reads)
        .unwrap();
    assert_eq!(items[0].name, "cache");
    assert!(more);
    assert_eq!(reads.nodes_read(), 1);
    eprintln!(
        "page preparation payload authorizer callbacks={}",
        payload_reads.load(Ordering::Relaxed)
    );
    let error = store
        .navigation_children_with_budget("probe-navigation", 1, 1, 1, &mut budget(1, 4 << 20))
        .unwrap_err();
    assert!(
        matches!(error, StoreError::Json(_)),
        "selected legacy error disappeared: {error}"
    );
    // 编译期 authorizer 计数不是物理列读取量。独立准备实际 production 常量探针，
    // 拒绝所有 payload 列引用；正控制证明同一 hook 确实拦截了 payload SQL。
    store
        .connection
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Read {
                    table_name: "nodes",
                    column_name: "node_json"
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    assert!(
        store
            .connection
            .prepare("SELECT node_json FROM nodes WHERE snapshot_id='probe-navigation' AND id=3")
            .is_err()
    );
    assert!(
        store
            .navigation_continuation_with_budget("probe-navigation", 1, 1, &mut budget(1, 1024))
            .unwrap()
    );
}
