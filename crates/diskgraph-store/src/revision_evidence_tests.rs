//! EV-05 所选批次、原始预算与窄查询回归。
use crate::tests::graph;
use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::{QueryBudget, QueryReadBudget, query_deadline};
use rusqlite::params;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn reads(edges: usize, bytes: usize) -> QueryReadBudget {
    let budget = QueryBudget {
        max_edges: edges,
        max_response_bytes: bytes,
        ..QueryBudget::default()
    };
    QueryReadBudget::new(budget, query_deadline(budget).unwrap()).unwrap()
}

fn fixture() -> SqliteSnapshotStore {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.save(&graph("snapshot", 100)).unwrap();
    store.connection.execute("INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation) VALUES ('old','snapshot',1,10)",[]).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation) VALUES ('new','snapshot',2,10)",
            [],
        )
        .unwrap();
    for run in ["old-run", "new-run", "unbound"] {
        store
            .connection
            .execute(
                "INSERT INTO collector_runs VALUES (?1,'snapshot','test',1,1,1,1,'{}',10)",
                [run],
            )
            .unwrap();
    }
    store.connection.execute_batch("INSERT INTO revision_runs VALUES ('old','old-run','active'); INSERT INTO revision_runs VALUES ('new','new-run','active'); INSERT INTO revision_runs VALUES ('new','old-run','dependency_only');").unwrap();
    add_entity(&store, "source", "old-run");
    add_entity(&store, "new-only", "new-run");
    add_entity(&store, "unbound-only", "unbound");
    add_edge(&store, "a", "old-run", "source", "target");
    add_edge(&store, "b", "new-run", "source", "target");
    add_edge(&store, "c", "unbound", "source", "target");
    for (id, run) in [
        ("old-evidence", "old-run"),
        ("new-evidence", "new-run"),
        ("hidden-evidence", "unbound"),
    ] {
        let json = serde_json::json!({"evidence_id":id,"run_id":run,"basis":"test","observed_at_unix_ms":1,"expires_at_unix_ms":null,"confidence":100,"input_fingerprint":"fingerprint"}).to_string();
        store
            .connection
            .execute(
                "INSERT INTO evidence_records VALUES ('snapshot',?1,?2,?3)",
                params![id, run, json],
            )
            .unwrap();
    }
    store
        .connection
        .execute(
            "UPDATE graph_revisions SET selection_sealed=1,evidence_complete=1",
            [],
        )
        .unwrap();
    store
}

fn add_entity(store: &SqliteSnapshotStore, id: &str, run: &str) {
    let json = serde_json::json!({"entity_id":id,"kind":"resource","identity":"{}","display":id,"source_run_id":run}).to_string();
    store
        .connection
        .execute(
            "INSERT INTO entities VALUES ('snapshot',?1,'resource',?2)",
            params![id, json],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO entity_run_memberships VALUES ('snapshot',?1,?2)",
            params![run, id],
        )
        .unwrap();
}

fn add_edge(store: &SqliteSnapshotStore, id: &str, run: &str, source: &str, target: &str) {
    let json = serde_json::json!({"edge_id":id,"source_entity_id":source,"target_entity_id":target,"relation":"protected_by","assertion_kind":"observed","evidence_refs":[["old-evidence","supports"]]}).to_string();
    store
        .connection
        .execute(
            "INSERT INTO relations VALUES ('snapshot',?1,?2,'protected_by',?3,?4)",
            params![id, source, target, json],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO relation_run_memberships VALUES ('snapshot',?1,?2)",
            params![run, id],
        )
        .unwrap();
}

#[test]
fn revision_evidence_selects_active_assertions_and_dependency_provenance() {
    let store = fixture();
    let old = store.revision_evidence("old").unwrap();
    let new = store.revision_evidence("new").unwrap();
    assert_eq!(old.revision_id(), "old");
    assert_eq!(old.snapshot_id(), "snapshot");
    assert_eq!(
        old.all_edges()
            .unwrap()
            .iter()
            .map(|e| e.edge_id.as_str())
            .collect::<Vec<_>>(),
        ["a"]
    );
    assert_eq!(
        new.edges_from("source", None)
            .unwrap()
            .iter()
            .map(|e| e.edge_id.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
    assert_eq!(new.edges_to("target", None).unwrap().len(), 1);
    assert!(old.entity("new-only").unwrap().is_none());
    assert!(new.entity("source").unwrap().is_some());
    assert!(new.entity("unbound-only").unwrap().is_none());
    assert!(
        new.entity_with_budget("unknown", &mut reads(2, 4096))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        new.evidence_for_edges(&["b".into()]).unwrap()[0].run_id,
        "old-run"
    );
    assert!(new.evidence_for_edges(&["a".into()]).is_err());
    assert!(
        new.evidence_record_with_budget("hidden-evidence", &mut reads(2, 4096))
            .unwrap()
            .is_none()
    );
    assert!(!new.has_incomplete_membership().unwrap());
    store.connection.execute("INSERT INTO collector_membership_diagnostics VALUES ('snapshot','edge','unknown','legacy_membership_unavailable_recollect')", []).unwrap();
    assert!(
        !new.has_incomplete_membership().unwrap(),
        "confirmed revision must not inherit unrelated later diagnostics"
    );
    assert!(matches!(
        store.revision_evidence("missing"),
        Err(StoreError::RevisionNotFound(_))
    ));
}

#[test]
fn revision_evidence_keyset_deduplicates_self_loop_and_multiple_memberships() {
    let store = fixture();
    add_edge(&store, "d", "new-run", "source", "source");
    add_edge(&store, "e", "new-run", "incoming", "source");
    store
        .connection
        .execute(
            "INSERT INTO relation_run_memberships VALUES ('snapshot','old-run','b')",
            [],
        )
        .unwrap();
    let reader = store.revision_evidence("new").unwrap();
    let mut budget = reads(5, 8192);
    let (first, more) = reader
        .edges_with_budget_page("source", None, None, None, 2, &mut budget)
        .unwrap();
    assert!(more);
    assert_eq!(
        first.iter().map(|e| e.edge_id.as_str()).collect::<Vec<_>>(),
        ["b", "d"]
    );
    let (last, more) = reader
        .edges_with_budget_page("source", None, None, Some("d"), 2, &mut budget)
        .unwrap();
    assert!(!more);
    assert_eq!(last[0].edge_id, "e");
    assert_eq!(budget.remaining_edges(), 2);
    assert_eq!(
        reader.edges_to_page("source", None, 1).unwrap().0[0].edge_id,
        "d"
    );
}

#[test]
fn revision_evidence_ignores_cross_snapshot_members_even_with_corrupt_binding() {
    let mut store = fixture();
    store
        .publish_revision("other-job", &graph("other", 100), "other-revision", 3)
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO collector_runs VALUES ('foreign','other','test',1,1,1,1,'{}',10)",
            [],
        )
        .unwrap();
    store.connection.execute("INSERT INTO graph_revisions(revision_id,snapshot_id,published_at_unix_ms,writer_generation) VALUES ('corrupt','snapshot',3,10)",[]).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO revision_runs VALUES ('corrupt','foreign','active')",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO revision_runs VALUES ('corrupt','old-run','active')",
            [],
        )
        .unwrap();
    store.connection.execute("INSERT INTO relations VALUES ('other','foreign-edge','source','protected_by','target','bad')", []).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO relation_run_memberships VALUES ('other','foreign','foreign-edge')",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO relation_run_memberships VALUES ('snapshot','foreign','b')",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .revision_evidence("corrupt")
            .unwrap()
            .all_edges()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn revision_evidence_budget_precedes_bad_json_and_lookahead_decode() {
    let store = fixture();
    add_edge(&store, "d", "new-run", "source", "target");
    store
        .connection
        .execute(
            "UPDATE relations SET edge_json=?1 WHERE edge_id IN ('a','c','d')",
            [vec![255u8; 100_000]],
        )
        .unwrap();
    let reader = store.revision_evidence("new").unwrap();
    assert_eq!(
        reader.edges_from_page("source", None, 1).unwrap().0.len(),
        1
    );
    let mut budget = reads(1, 4096);
    assert_eq!(
        reader
            .edges_with_budget_page("source", Some(true), None, None, 1, &mut budget)
            .unwrap()
            .0
            .len(),
        1
    );
    assert!(matches!(
        reader.edges_with_budget_page(
            "source",
            Some(true),
            None,
            Some("b"),
            1,
            &mut reads(2, 4096)
        ),
        Err(StoreError::BudgetExceeded)
    ));
    store
        .connection
        .execute(
            "UPDATE relations SET edge_json=CAST(edge_json AS BLOB) WHERE edge_id='b'",
            [],
        )
        .unwrap();
    assert!(matches!(
        reader.edges_text_with_budget_page(
            "source",
            Some(true),
            None,
            None,
            1,
            &mut reads(2, 4096)
        ),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(
        reader
            .edges_with_budget_page("source", Some(true), None, None, 1, &mut reads(2, 4096))
            .unwrap()
            .0
            .len(),
        1
    );
    store
        .connection
        .execute(
            "UPDATE entities SET entity_json=?1 WHERE entity_id='source'",
            ["x".repeat(100_000)],
        )
        .unwrap();
    assert!(matches!(
        reader.entity_with_budget("source", &mut reads(2, 4096)),
        Err(StoreError::BudgetExceeded)
    ));
}

#[test]
fn revision_evidence_narrow_query_does_not_scan_200k_unrelated_memberships() {
    let store = fixture();
    store.connection.execute_batch("WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<200000) INSERT INTO relations SELECT 'snapshot','bulk-'||n,'unrelated','protected_by','unrelated-target','bad' FROM seq;
    INSERT INTO relation_run_memberships SELECT snapshot_id,'unbound',edge_id FROM relations WHERE edge_id LIKE 'bulk-%';").unwrap();
    let ticks = Arc::new(AtomicUsize::new(0));
    let observed = ticks.clone();
    store
        .connection
        .progress_handler(
            1000,
            Some(move || observed.fetch_add(1, Ordering::Relaxed) > 100),
        )
        .unwrap();
    let reader = store.revision_evidence("new").unwrap();
    let edges = reader
        .edges_with_budget_page("source", None, None, None, 2, &mut reads(2, 4096))
        .unwrap()
        .0;
    assert_eq!(edges.len(), 1);
    assert!(
        reader
            .entity_with_budget("source", &mut reads(2, 4096))
            .unwrap()
            .is_some()
    );
    assert!(!reader.has_incomplete_membership().unwrap());
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    assert!(
        ticks.load(Ordering::Relaxed) < 100,
        "narrow read exceeded 100k VM instructions"
    );
}

#[test]
fn revision_evidence_shares_entity_and_provenance_raw_budget() {
    let store = fixture();
    let entity_bytes = usize::try_from(
        store
            .connection
            .query_row(
                "SELECT length(CAST(entity_json AS BLOB)) FROM entities WHERE entity_id='source'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
    )
    .unwrap();
    let evidence_bytes = usize::try_from(store.connection.query_row(
        "SELECT length(CAST(evidence_json AS BLOB)) FROM evidence_records WHERE evidence_id='old-evidence'", [], |row| row.get::<_, i64>(0)
    ).unwrap()).unwrap();
    let reader = store.revision_evidence("new").unwrap();
    let mut exact = reads(2, entity_bytes + evidence_bytes);
    assert!(
        reader
            .entity_with_budget("source", &mut exact)
            .unwrap()
            .is_some()
    );
    assert!(
        reader
            .evidence_record_with_budget("old-evidence", &mut exact)
            .unwrap()
            .is_some()
    );
    assert_eq!(exact.remaining_raw_bytes(), 0);
    let mut short = reads(2, entity_bytes + evidence_bytes - 1);
    reader.entity_with_budget("source", &mut short).unwrap();
    assert!(matches!(
        reader.evidence_record_with_budget("old-evidence", &mut short),
        Err(StoreError::BudgetExceeded)
    ));
    store
        .connection
        .execute(
            "UPDATE evidence_records SET evidence_json=?1 WHERE evidence_id='old-evidence'",
            ["x".repeat(100_000)],
        )
        .unwrap();
    assert!(matches!(
        reader.evidence_record_with_budget("old-evidence", &mut reads(2, 4096)),
        Err(StoreError::BudgetExceeded)
    ));
    store
        .connection
        .execute(
            "UPDATE evidence_records SET evidence_json='bad' WHERE evidence_id='old-evidence'",
            [],
        )
        .unwrap();
    assert!(matches!(
        reader.evidence_record_with_budget("old-evidence", &mut reads(2, 4096)),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn revision_evidence_expired_shared_deadline_is_not_complete_empty() {
    let store = fixture();
    let reader = store.revision_evidence("new").unwrap();
    let deadline = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_millis(1))
        .unwrap();
    let mut budget = QueryReadBudget::new(QueryBudget::default(), deadline).unwrap();
    assert!(matches!(
        reader.edges_with_budget_page("unknown", None, None, None, 1, &mut budget),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        reader.entity_with_budget("unknown", &mut budget),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(matches!(
        reader.evidence_record_with_budget("unknown", &mut budget),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(
        budget.stopped(),
        Some(diskgraph_core::TruncationReason::Deadline)
    );
}
