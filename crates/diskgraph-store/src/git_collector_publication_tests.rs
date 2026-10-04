use crate::git_job_test_fixtures::{batch, fixture};
use crate::{SqliteSnapshotStore, StoreError};
use diskgraph_core::{GitEvidenceJobInput, QueryBudget, QueryReadBudget};
use rusqlite::params;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn count(store: &SqliteSnapshotStore, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn reads(bytes: usize, edges: usize) -> QueryReadBudget {
    QueryReadBudget::new(
        QueryBudget {
            max_nodes: 1,
            max_edges: edges,
            max_response_bytes: bytes,
            ..QueryBudget::default()
        },
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .unwrap()
}
#[test]
fn latest_base_cas_and_receipt_replay_never_duplicate_a_publication() {
    let (_, mut store, input, _) = fixture();
    let first = store
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "first",
            &batch(&input, "run-one"),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        store.job_publication_receipt("job-one").unwrap(),
        Some(first.clone())
    );
    let replay = store
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (9, 1002),
            "not-published",
            &batch(&input, "new-generation"),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(count(&store, "collector_runs"), 1);
    assert_eq!(count(&store, "graph_revisions"), 2);
    assert!(matches!(
        store.publish_git_collector_revision_checked(
            "different-job",
            &input,
            (1, 1003),
            "stale",
            &batch(&input, "stale-run"),
            || Ok(())
        ),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(count(&store, "job_publication_receipts"), 1);
    assert!(store.revision("not-published").is_err());
}
#[test]
fn actual_last_receipt_sql_is_rolled_back_when_terminal_check_refuses() {
    let (_, mut store, input, _) = fixture();
    let observed = Arc::new(AtomicBool::new(false));
    let signal = observed.clone();
    store
        .connection
        .update_hook(Some(
            move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if action == rusqlite::hooks::Action::SQLITE_INSERT
                    && table == "job_publication_receipts"
                {
                    signal.store(true, Ordering::SeqCst);
                }
            },
        ))
        .unwrap();
    let result = store.publish_git_collector_revision_checked(
        "job-one",
        &input,
        (1, 1001),
        "refused",
        &batch(&input, "run-one"),
        || {
            if observed.load(Ordering::SeqCst) {
                Err(StoreError::Conflict("terminal refusal".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(result, Err(StoreError::Conflict(_))));
    assert!(observed.load(Ordering::SeqCst));
    for table in [
        "collector_runs",
        "entities",
        "relations",
        "evidence_records",
        "revision_runs",
        "job_publication_receipts",
    ] {
        assert_eq!(count(&store, table), 0, "{table}");
    }
    assert_eq!(count(&store, "graph_revisions"), 1);
    assert_eq!(count(&store, "revision_ownership"), 1);
    assert_eq!(
        store
            .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
            .unwrap()
            .as_deref(),
        Some("base")
    );
}
#[test]
fn same_target_demotes_only_old_git_assertions_and_keeps_other_target_active() {
    let (_, mut store, input, _) = fixture();
    store
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "first",
            &batch(&input, "one"),
            || Ok(()),
        )
        .unwrap();
    let other = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        "first".into(),
        1,
        input.limits().clone(),
    )
    .unwrap();
    store
        .publish_git_collector_revision_checked(
            "job-other",
            &other,
            (1, 1002),
            "second",
            &batch(&other, "other"),
            || Ok(()),
        )
        .unwrap();
    let fresh = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        "second".into(),
        2,
        input.limits().clone(),
    )
    .unwrap();
    store
        .publish_git_collector_revision_checked(
            "job-fresh",
            &fresh,
            (1, 1003),
            "third",
            &batch(&fresh, "fresh"),
            || Ok(()),
        )
        .unwrap();
    let selection: Vec<(String, String)> = store
        .connection
        .prepare("SELECT run_id,role FROM revision_runs WHERE revision_id='third' ORDER BY run_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        selection,
        vec![
            ("fresh".into(), "active".into()),
            ("one".into(), "dependency_only".into()),
            ("other".into(), "active".into())
        ]
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT role FROM revision_runs WHERE revision_id='first' AND run_id='one'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "active"
    );
    assert_eq!(
        store.revision("base").unwrap().snapshot_id,
        store.revision("third").unwrap().snapshot_id
    );
    assert_eq!(count(&store, "snapshots"), 1);
}
#[test]
fn bounded_selection_rejects_large_legal_run_json_before_decoding() {
    let (_, mut store, input, _) = fixture();
    let mut old = crate::collector_publication_tests::batch("old", "snapshot");
    old.run.errors = vec!["x".repeat(2 << 20)];
    store
        .publish_collector_revision(
            "base",
            "large",
            1001,
            (input.server_id().as_str(), input.scope_id().as_str()),
            &old,
            &[("old", "active")],
        )
        .unwrap();
    assert!(matches!(
        store.git_collector_selection_with_budget("large", 2, &mut reads(4096, 2)),
        Err(StoreError::BudgetExceeded)
    ));
    let fresh = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        "large".into(),
        2,
        input.limits().clone(),
    )
    .unwrap();
    assert!(matches!(
        store.publish_git_collector_revision_checked(
            "job-new",
            &fresh,
            (1, 1002),
            "not-published",
            &batch(&fresh, "new"),
            || Ok(())
        ),
        Err(StoreError::BudgetExceeded)
    ));
    assert_eq!(count(&store, "collector_runs"), 1);
    assert_eq!(count(&store, "job_publication_receipts"), 0);
}
#[test]
fn a_broken_source_closure_and_unsafe_summary_cannot_be_published_as_complete() {
    let (_, mut store, input, _) = fixture();
    let old = crate::collector_publication_tests::batch("old", "snapshot");
    store
        .publish_collector_revision(
            "base",
            "old-revision",
            1001,
            (input.server_id().as_str(), input.scope_id().as_str()),
            &old,
            &[("old", "active")],
        )
        .unwrap();
    store.connection.execute("UPDATE entities SET entity_json=json_set(entity_json,'$.source_run_id','missing') WHERE entity_id='resource'",[]).unwrap();
    let fresh = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        "old-revision".into(),
        2,
        input.limits().clone(),
    )
    .unwrap();
    assert!(matches!(
        store.publish_git_collector_revision_checked(
            "job-new",
            &fresh,
            (1, 1002),
            "not-published",
            &batch(&fresh, "new"),
            || Ok(())
        ),
        Err(StoreError::InvalidGraph(_))
    ));
    assert_eq!(count(&store, "collector_runs"), 1);
    assert_eq!(count(&store, "job_publication_receipts"), 0);
    let (_, mut clean, input, _) = fixture();
    let mut unsafe_batch = batch(&input, "unsafe");
    unsafe_batch.evidence[0].basis = serde_json::json!({"head":"SECRET"}).to_string();
    assert!(
        clean
            .publish_git_collector_revision_checked(
                "unsafe-job",
                &input,
                (1, 100),
                "unsafe",
                &unsafe_batch,
                || Ok(())
            )
            .is_err()
    );
    assert_eq!(count(&clean, "collector_runs"), 0);
}
#[test]
fn ordinary_trusted_scan_publication_conflicts_with_the_fixed_git_base() {
    let (_, mut store, input, _) = fixture();
    let scan = crate::tests::graph("different-snapshot", 200);
    store.append_staging_nodes("scan", &scan.nodes).unwrap();
    store
        .publish_revision_owned(
            "scan",
            &scan,
            "scan",
            200,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    assert!(matches!(
        store.publish_git_collector_revision_checked(
            "job-git",
            &input,
            (1, 201),
            "git",
            &batch(&input, "git"),
            || Ok(())
        ),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(count(&store, "job_publication_receipts"), 0);
    assert_eq!(
        store.revision("scan").unwrap().snapshot_id,
        "different-snapshot"
    );
}
#[test]
fn receipt_survives_result_pruning_and_rejects_mutation_or_old_protocol_writer() {
    let (_, mut store, input, _) = fixture();
    let receipt = store
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "first",
            &batch(&input, "one"),
            || Ok(()),
        )
        .unwrap();
    assert!(
        store
            .connection
            .execute("UPDATE job_publication_receipts SET input_sha256='bad'", [])
            .is_err()
    );
    assert!(
        store
            .connection
            .execute("DELETE FROM job_publication_receipts", [])
            .is_err()
    );
    assert!(store.connection.execute("INSERT INTO job_publication_receipts(job_id,schema_version,input_sha256,server_id,scope_id,snapshot_id,revision_id,run_id,node_id,receipt_json,writer_generation) VALUES('obsolete',0,?1,'server','scope','snapshot','obsolete','obsolete',1,?2,0)",
        params![receipt.input_sha256(),serde_json::to_string(&receipt).unwrap()]).is_err());
    let scan = crate::tests::graph("new-snapshot", 2000);
    store.append_staging_nodes("new-scan", &scan.nodes).unwrap();
    store
        .publish_revision_owned(
            "new-scan",
            &scan,
            "latest",
            2000,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    let removed = store
        .prune_revisions(
            input.server_id().as_str(),
            input.scope_id().as_str(),
            1,
            true,
        )
        .unwrap();
    assert!(
        removed
            .iter()
            .any(|revision| revision.revision_id == "first")
    );
    assert!(store.revision("first").is_err());
    assert_eq!(count(&store, "collector_runs"), 0);
    assert_eq!(
        store.job_publication_receipt("job-one").unwrap(),
        Some(receipt.clone())
    );
    let replay = store
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (9, 3000),
            "duplicate",
            &batch(&input, "never-run"),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(replay, receipt);
    assert_eq!(count(&store, "collector_runs"), 0);
    assert_eq!(
        store
            .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
            .unwrap()
            .as_deref(),
        Some("latest")
    );
}
