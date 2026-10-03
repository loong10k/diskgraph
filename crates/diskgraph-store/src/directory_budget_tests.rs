//! 有界目录页的实际解码与索引工作量；来源：D26 / Q-02。
use crate::child_aggregate_tests::wide_store;
use diskgraph_core::{QueryBudget, QueryReadBudget, TruncationReason};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn budget(nodes: usize) -> QueryReadBudget {
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
fn wide_directory_decoding_scales_with_the_page_and_not_the_snapshot() {
    let store = wide_store(false);
    let steps = Arc::new(AtomicUsize::new(0));
    let count = steps.clone();
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                count.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    for limit in [1, 5] {
        steps.store(0, Ordering::Relaxed);
        let mut reads = budget(limit);
        let (items, more, unknown) = store
            .children_with_budget("wide", 1, 0, limit as u64, false, &mut reads)
            .unwrap();
        assert_eq!(items.len(), limit);
        assert_eq!(reads.nodes_read(), limit);
        assert_eq!(unknown, 0);
        assert!(more);
        assert!(reads.stopped().is_none());
        let actual = steps.load(Ordering::Relaxed);
        eprintln!(
            "directory page={limit}, nodes_read={}, VM steps={actual}",
            reads.nodes_read()
        );
        assert!(actual < 1000, "bounded page walked siblings: {actual}");
    }
}

#[test]
fn known_page_skips_unknown_siblings_without_decoding_their_payloads() {
    let store = wide_store(true);
    let steps = Arc::new(AtomicUsize::new(0));
    let count = steps.clone();
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                count.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    let mut reads = budget(1);
    let (items, more, unknown) = store
        .children_with_budget("wide", 1, 0, 1, true, &mut reads)
        .unwrap();
    assert_eq!(items[0].name, "cache");
    assert_eq!(unknown, 200_000);
    assert!(!more);
    assert_eq!(reads.nodes_read(), 1);
    let actual = steps.load(Ordering::Relaxed);
    eprintln!("known directory page=1, unknown=200000, VM steps={actual}");
    assert!(
        actual < 1000,
        "known page walked unknown siblings: {actual}"
    );
}

#[test]
fn node_budget_and_page_limit_do_not_decode_a_bad_continuation() {
    let store = wide_store(false);
    store
        .connection
        .execute("UPDATE nodes SET kind='broken' WHERE id=200001", [])
        .unwrap();
    let (page, more, _) = store
        .children_with_budget("wide", 1, 0, 1, false, &mut budget(10))
        .unwrap();
    assert_eq!(page.len(), 1);
    assert!(more);
    let mut reads = budget(1);
    let (page, more, _) = store
        .children_with_budget("wide", 1, 0, 2, false, &mut reads)
        .unwrap();
    assert_eq!(page.len(), 1);
    assert!(more);
    assert_eq!(reads.stopped(), Some(TruncationReason::NodeLimit));
    let error = store
        .children_with_budget("wide", 1, 0, 2, false, &mut budget(2))
        .unwrap_err();
    assert!(error.to_string().contains("unknown node kind"), "{error}");
}
