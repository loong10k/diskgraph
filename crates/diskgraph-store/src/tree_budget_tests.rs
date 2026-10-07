//! 树 minimum 过滤在真实宽目录中分别查已知/未知索引，并限制真正解码量。

use crate::{SqliteSnapshotStore, child_aggregate_tests, tests::graph};
use diskgraph_core::{NodeKind, QueryBudget, QueryReadBudget, ResourceLocator};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn sparse_unknown_store() -> SqliteSnapshotStore {
    let store = child_aggregate_tests::wide_store(false);
    let mut unknown = graph("unused", 0).nodes.remove(1);
    unknown.id = 200_002;
    unknown.kind = NodeKind::File;
    unknown.name = "child-000200000".to_owned();
    unknown.locator = ResourceLocator::NativePath("/tmp/child-000200000".to_owned());
    unknown.size_known = false;
    unknown.subtree_bytes = 300_000;
    store.connection.execute("UPDATE nodes SET subtree_bytes=300000,node_json=?1,kind=NULL WHERE snapshot_id='wide' AND id=200002", [serde_json::to_string(&unknown).unwrap()]).unwrap();
    super::directory_aggregates::rebuild(&store.connection, Some("wide")).unwrap();
    store
}

fn ledger(nodes: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_nodes: nodes,
            ..QueryBudget::default()
        },
        Instant::now().checked_add(Duration::from_secs(30)).unwrap(),
    )
    .unwrap()
}

#[test]
fn sparse_unknown_tree_page_seeks_without_visiting_the_known_prefix() {
    let store = sparse_unknown_store();
    let steps = child_aggregate_tests::count_steps(&store);
    let mut budget = ledger(1);
    let (nodes, more) = store
        .tree_children_with_budget("wide", 1, 200_001, 1, &mut budget)
        .unwrap();
    assert_eq!(nodes.len(), 1);
    assert!(!nodes[0].size_known);
    assert_eq!(nodes[0].id, 200_002);
    assert!(!more);
    assert_eq!(budget.nodes_read(), 1);
    assert_eq!(
        store.tree_child_counts("wide", 1, 200_001).unwrap(),
        (200_001, 1)
    );
    assert!(
        steps.load(Ordering::Relaxed) < 1500,
        "sparse unknown tree scanned excluded siblings: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}

#[test]
fn merged_tree_window_does_not_decode_a_corrupt_unknown_continuation() {
    let store = sparse_unknown_store();
    store
        .connection
        .execute(
            "UPDATE nodes SET subtree_bytes=400000 WHERE snapshot_id='wide' AND id=200001",
            [],
        )
        .unwrap();
    super::directory_aggregates::rebuild(&store.connection, Some("wide")).unwrap();
    // 旧 JSON 是该未知节点的真实解码来源；定位类型损坏但 JSON/排序字段有效。
    store.connection.execute("UPDATE nodes SET node_json=json_set(node_json,'$.locator.type','invalid_continuation') WHERE snapshot_id='wide' AND id=200002", []).unwrap();
    let steps = child_aggregate_tests::count_steps(&store);
    let mut budget = ledger(1);
    let (nodes, more) = store
        .tree_children_with_budget("wide", 1, 200_001, 1, &mut budget)
        .unwrap();
    assert_eq!(nodes[0].id, 200_001);
    assert!(more);
    assert_eq!(budget.nodes_read(), 1);
    assert_eq!(
        store.tree_child_counts("wide", 1, 200_001).unwrap(),
        (200_001, 2)
    );
    assert!(
        steps.load(Ordering::Relaxed) < 1500,
        "bounded merge scanned siblings: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
    // 增大同一查询的额度后确实会触及坏记录，证明负控制有效。
    assert!(
        store
            .tree_children_with_budget("wide", 1, 200_001, 2, &mut ledger(2))
            .is_err(),
        "in-budget malformed continuation must fail"
    );
}

#[test]
fn dense_mixed_unknown_tree_counts_do_not_walk_the_threshold_prefix() {
    let store = child_aggregate_tests::wide_store(true);
    let mut template = graph("unused", 0).nodes.remove(1);
    template.kind = NodeKind::File;
    template.size_known = false;
    // 全部未知记录都是可还原的完整旧 JSON；尺寸分布跨阈值，不借同尺寸捷径。
    store.connection.execute("UPDATE nodes SET subtree_bytes=id-2,node_json=json_set(?1,'$.id',id,'$.name',name,'$.locator.value',json_extract(locator_key,'$.value'),'$.subtree_bytes',id-2),kind=NULL WHERE snapshot_id='wide' AND id>2", [serde_json::to_string(&template).unwrap()]).unwrap();
    super::directory_aggregates::rebuild(&store.connection, Some("wide")).unwrap();
    let steps = child_aggregate_tests::count_steps(&store);
    let (all, kept) = store.tree_child_counts("wide", 1, 100_001).unwrap();
    assert_eq!(all, 200_001);
    assert_eq!(
        kept, 100_000,
        "minimum retains the original numeric size filter"
    );
    assert!(
        steps.load(Ordering::Relaxed) < 1500,
        "dense unknown exact tree count walked siblings: {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}

#[test]
fn tree_sql_window_scales_with_remaining_budget_not_requested_limit() {
    let store = child_aggregate_tests::wide_store(false);
    let steps = child_aggregate_tests::count_steps(&store);
    let mut budget = ledger(3);
    // 整次请求已消耗一个节点，调用方的大页不能重新获得完整预算。
    assert!(budget.admit(1, 0, 0));
    let (nodes, more) = store
        .tree_children_with_budget("wide", 1, 0, 5_000, &mut budget)
        .unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(budget.nodes_read(), 3);
    assert!(more);
    assert_eq!(
        budget.stopped(),
        Some(diskgraph_core::TruncationReason::NodeLimit)
    );
    let actual = steps.load(Ordering::Relaxed);
    eprintln!("tree remaining_nodes=2, requested=5000, VM steps={actual}");
    assert!(
        actual < 1_500,
        "tree SQL window ignored remaining budget: {actual} VM steps"
    );
}
