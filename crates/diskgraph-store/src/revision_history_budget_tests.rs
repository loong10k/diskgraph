//! 相同实体反复采集后的页面成本必须独立于 inactive 历史数量。
use crate::collector_publication_tests::{base, batch};
use diskgraph_core::{CollectorRun, QueryBudget, QueryReadBudget, query_deadline};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn selected_page_does_not_scan_same_entity_inactive_history() {
    for historical_edges in [20_000, 200_000] {
        let mut store = base();
        let current = batch("current", "snap");
        store
            .publish_collector_revision(
                "base",
                "current",
                2,
                ("server", "scope"),
                &current,
                &[("current", "active")],
            )
            .unwrap();
        let inactive = CollectorRun {
            run_id: "inactive".into(),
            snapshot_id: "snap".into(),
            collector_id: "fixture".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: 1,
            coverage_complete: true,
            errors: vec![],
        };
        store
            .record_collector_batch("snap", &inactive, &[], &[], &[])
            .unwrap();
        store.connection.execute(
            "WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<?1)
             INSERT INTO relations SELECT 'snap',printf('0-history-%09d',n),'resource','owned_by_project','other','bad' FROM seq",
            [historical_edges]).unwrap();
        store.connection.execute("INSERT INTO relation_run_memberships SELECT snapshot_id,'inactive',edge_id FROM relations WHERE edge_id LIKE '0-history-%'",[]).unwrap();
        let ticks = Arc::new(AtomicUsize::new(0));
        let observed = ticks.clone();
        store
            .connection
            .progress_handler(
                1,
                Some(move || observed.fetch_add(1, Ordering::Relaxed) >= 100_000),
            )
            .unwrap();
        let budget = QueryBudget::default();
        let mut reads = QueryReadBudget::new(budget, query_deadline(budget).unwrap()).unwrap();
        let result = store
            .revision_evidence("current")
            .unwrap()
            .edges_with_budget_page("resource", Some(true), None, None, 1, &mut reads);
        store
            .connection
            .progress_handler(0, None::<fn() -> bool>)
            .unwrap();
        eprintln!(
            "inactive same-entity edges={historical_edges}, VM instructions {}, query_ok={}",
            ticks.load(Ordering::Relaxed),
            result.is_ok()
        );
        let (page, more) = result.expect("one selected edge must not scan inactive history");
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].edge_id, "edge-current");
        assert!(!more);
        assert!(
            ticks.load(Ordering::Relaxed) < 100_000,
            "one-page query exceeded 100k VM instructions"
        );
    }
}
