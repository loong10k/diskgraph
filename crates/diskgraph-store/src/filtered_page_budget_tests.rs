//! 当前预算页在真实200k隔离SQLite夹具上的实际准入数和VM工作量。
use crate::child_aggregate_tests::{count_steps, wide_store};
use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::{QueryBudget, QueryReadBudget};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn ledger(nodes: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_nodes: nodes,
            ..QueryBudget::default()
        },
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap()
}

#[test]
fn actual_filtered_pages_keep_the_range_index_and_decode_only_the_page() {
    let store = wide_store(false);
    let steps = count_steps(&store);
    for limit in [1, 5] {
        steps.store(0, Ordering::Relaxed);
        let mut reads = ledger(limit);
        let (items, next, unknown) = store
            .children_page_with_budget("wide", 1, Some(100_000), 0, limit as u64, &mut reads)
            .unwrap();
        assert_eq!(items.len(), limit);
        assert_eq!(reads.nodes_read(), limit);
        assert!(next.is_some());
        assert_eq!(unknown, 0);
        let actual = steps.load(Ordering::Relaxed);
        eprintln!(
            "DG_FILTERED_PAGE limit={limit} nodes={} vm_steps_quantized={actual}",
            reads.nodes_read()
        );
        assert!(
            actual < 1000,
            "filtered page walked 200k siblings: {actual}"
        );
    }
    // 旧v8宽目录夹具的手工插入节点没有搜索行；沿既有keyset夹具补充其ASCII规范字段。
    store.connection.execute("INSERT INTO node_search(snapshot_id,id,name_fold,path_fold) SELECT snapshot_id,id,name,name FROM nodes WHERE snapshot_id='wide' AND id>2", []).unwrap();
    // 搜索的子串过滤可能检查更多名称，实际解码仍只随页变化；不声称任意substring为O(page)。
    for limit in [1, 5] {
        steps.store(0, Ordering::Relaxed);
        let mut reads = ledger(limit);
        let (items, more) = store
            .search_page_with_budget("wide", "child-", None, 0, limit as u64, &mut reads)
            .unwrap();
        assert_eq!(items.len(), limit);
        assert_eq!(reads.nodes_read(), limit);
        assert!(more);
        eprintln!(
            "DG_SEARCH_PAGE limit={limit} nodes={} vm_steps_quantized={}",
            reads.nodes_read(),
            steps.load(Ordering::Relaxed)
        );
    }
}

#[test]
fn actual_known_filter_does_not_decode_200k_unknown_siblings() {
    let store = wide_store(true);
    let steps = count_steps(&store);
    let mut reads = ledger(1);
    let (items, next, unknown) = store
        .children_page_with_budget("wide", 1, None, 0, 1, &mut reads)
        .unwrap();
    assert_eq!(items[0].name, "cache");
    assert_eq!(reads.nodes_read(), 1);
    assert!(next.is_none());
    assert_eq!(unknown, 200_000);
    let actual = steps.load(Ordering::Relaxed);
    eprintln!("DG_FILTERED_KNOWN unknown={unknown} nodes=1 vm_steps_quantized={actual}");
    assert!(
        actual < 1000,
        "known page scanned unknown siblings: {actual}"
    );
}

#[test]
fn search_budget_keeps_unicode_final_sigma_and_offset_keyset_semantics() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let mut graph = crate::tests::graph("unicode", 100);
    graph.nodes[1].name = "ÄΟΣ".into();
    graph.nodes[1].locator =
        diskgraph_core::ResourceLocator::NativePath("/tmp/diskgraph-test/ÄΟΣ".into());
    store.save(&graph).unwrap();
    let (items, more) = store
        .search_page_with_budget("unicode", "äος", None, 0, 1, &mut ledger(1))
        .unwrap();
    assert_eq!(items[0].name, "ÄΟΣ");
    assert!(!more);
    let (empty, more) = store
        .search_page_with_budget(
            "unicode",
            "",
            Some((items[0].name.as_str(), items[0].id)),
            999,
            1,
            &mut ledger(1),
        )
        .unwrap();
    assert!(empty.is_empty());
    assert!(!more);
}

#[test]
fn search_budget_does_not_decode_a_bad_continuation_and_rejects_expired_ledger() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&crate::tests::graph("page", 100)).unwrap();
    store
        .connection
        .execute("UPDATE nodes SET kind='broken' WHERE id=1", [])
        .unwrap();
    let (items, more) = store
        .search_page_with_budget("page", "", None, 0, 1, &mut ledger(1))
        .unwrap();
    assert_eq!(items[0].id, 2);
    assert!(more); // cache先于diskgraph-test，后者只作探针。
    assert!(matches!(
        store.search_page_with_budget("page", "", None, 0, 2, &mut ledger(2)),
        Err(StoreError::InvalidGraph(_))
    ));
    let mut expired = QueryReadBudget::new(
        QueryBudget::default(),
        Instant::now() - Duration::from_millis(1),
    )
    .unwrap();
    assert!(matches!(
        store.search_page_with_budget("page", "", None, 0, 1, &mut expired),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(expired.nodes_read(), 0);
}
