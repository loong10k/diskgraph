//! 精确 locator 读取工作量与共享额度回归；来源：D25 / Q-02。
use crate::{SqliteSnapshotStore, child_aggregate_tests};
use diskgraph_core::{QueryBudget, QueryReadBudget, ResourceLocator};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[test]
fn exact_locator_seeks_in_a_wide_revision_without_scanning_siblings() {
    let store: SqliteSnapshotStore = child_aggregate_tests::wide_store(false);
    // 每步计数，避免把每 100 步采样的余量误认为精确上限。
    let steps = Arc::new(AtomicUsize::new(0));
    let counter = steps.clone();
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                counter.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let mut budget = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap();
    let node = store
        .node_by_locator_with_budget(
            "wide",
            &ResourceLocator::NativePath("/tmp/child-000200000".into()),
            &mut budget,
        )
        .unwrap()
        .unwrap();
    eprintln!("exact locator VM steps: {}", steps.load(Ordering::Relaxed));
    assert_eq!(node.id, 200_002);
    assert_eq!(budget.nodes_read(), 1);
    assert!(
        steps.load(Ordering::Relaxed) < 500,
        "point lookup executed {} VM steps",
        steps.load(Ordering::Relaxed)
    );
}

#[test]
fn indexed_locator_preserves_type_unicode_and_legacy_unknown_size() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = crate::tests::graph("legacy-locator", 100);
    graph.nodes[1].locator = ResourceLocator::NativePath("/tmp/中文\\n\\\"path".into());
    graph.nodes[1].size_known = false;
    store.save(&graph).unwrap();
    store
        .connection
        .execute(
            "UPDATE nodes SET kind=NULL,node_json=?1 WHERE snapshot_id=?2 AND id=?3",
            rusqlite::params![
                serde_json::to_string(&graph.nodes[1]).unwrap(),
                graph.snapshot.id,
                i64::try_from(graph.nodes[1].id).unwrap()
            ],
        )
        .unwrap();
    let mut budget = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap();
    let wrong_type = ResourceLocator::DocumentUri("/tmp/中文\\n\\\"path".into());
    assert!(
        store
            .node_by_locator_with_budget(&graph.snapshot.id, &wrong_type, &mut budget)
            .unwrap()
            .is_none()
    );
    assert_eq!(budget.nodes_read(), 0);
    let node = store
        .node_by_locator_with_budget(&graph.snapshot.id, &graph.nodes[1].locator, &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(node, graph.nodes[1]);
    assert!(!node.size_known);
    assert_eq!(budget.nodes_read(), 1);
}
