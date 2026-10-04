use crate::StoreError;
use crate::process_job_test_fixtures::{batch, count, fixture, next};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[test]
fn publication_is_atomic_replayable_and_only_changes_the_new_revision() {
    let (_, mut store, input, _) = fixture();
    let b = batch(&input, "positive", true, false);
    let receipt = store
        .publish_process_collector_revision_checked(
            "process-job",
            &input,
            (1, 1001),
            "first",
            &b,
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        store
            .process_job_publication_receipt("process-job")
            .unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(count(&store, "collector_runs"), 1);
    assert!(
        store
            .revision_evidence("process-base")
            .unwrap()
            .all_edges()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .revision_evidence("first")
            .unwrap()
            .all_edges()
            .unwrap(),
        b.edges
    );
    assert_eq!(
        store
            .publish_process_collector_revision_checked(
                "process-job",
                &input,
                (2, 1002),
                "no-replay",
                &batch(&input, "no-replay", true, false),
                || Ok(())
            )
            .unwrap(),
        receipt
    );
    assert_eq!(count(&store, "collector_runs"), 1);
    assert!(store.revision("no-replay").is_err());
    assert!(matches!(
        store.publish_process_collector_revision_checked(
            "other-job",
            &input,
            (1, 1003),
            "stale",
            &batch(&input, "stale", true, false),
            || Ok(())
        ),
        Err(StoreError::Conflict(_))
    ));
}
#[test]
fn actual_last_receipt_sql_refusal_rolls_back_every_publication_row() {
    let (_, mut store, input, _) = fixture();
    let written = Arc::new(AtomicBool::new(false));
    let mark = written.clone();
    store
        .connection
        .update_hook(Some(
            move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if action == rusqlite::hooks::Action::SQLITE_INSERT
                    && table == "process_job_publication_receipts"
                {
                    mark.store(true, Ordering::SeqCst);
                }
            },
        ))
        .unwrap();
    let result = store.publish_process_collector_revision_checked(
        "process-job",
        &input,
        (1, 1001),
        "denied",
        &batch(&input, "denied", true, false),
        || {
            if written.load(Ordering::SeqCst) {
                Err(StoreError::Conflict("terminal denial".into()))
            } else {
                Ok(())
            }
        },
    );
    assert!(written.load(Ordering::SeqCst));
    assert!(matches!(result, Err(StoreError::Conflict(_))));
    assert_eq!(count(&store, "graph_revisions"), 1);
    for table in [
        "collector_runs",
        "entities",
        "evidence_records",
        "relations",
        "process_job_publication_receipts",
    ] {
        assert_eq!(count(&store, table), 0);
    }
    assert_eq!(
        store
            .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
            .unwrap()
            .as_deref(),
        Some("process-base")
    );
}
#[test]
fn partial_and_complete_empty_refreshes_keep_old_positive_active() {
    let (_, mut store, input, _) = fixture();
    let positive = batch(&input, "positive", true, false);
    store
        .publish_process_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "first",
            &positive,
            || Ok(()),
        )
        .unwrap();
    let partial = next(&input, "first");
    store
        .publish_process_collector_revision_checked(
            "job-two",
            &partial,
            (1, 1002),
            "partial",
            &batch(&partial, "partial", false, true),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        store
            .revision_evidence("partial")
            .unwrap()
            .all_edges()
            .unwrap(),
        positive.edges
    );
    let empty = next(&input, "partial");
    store
        .publish_process_collector_revision_checked(
            "job-three",
            &empty,
            (1, 1003),
            "empty",
            &batch(&empty, "empty", false, false),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        store
            .revision_evidence("empty")
            .unwrap()
            .all_edges()
            .unwrap(),
        positive.edges
    );
    let role: String = store
        .connection
        .query_row(
            "SELECT role FROM revision_runs WHERE revision_id='empty' AND run_id='positive'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(role, "active");
    assert_eq!(
        store
            .revision_evidence("first")
            .unwrap()
            .all_edges()
            .unwrap(),
        positive.edges
    );
    assert_eq!(count(&store, "process_job_publication_receipts"), 3);
}
#[test]
fn malformed_process_relation_and_missing_scan_epoch_refuse_publication() {
    let (_, mut store, input, _) = fixture();
    let mut wrong = batch(&input, "wrong", true, false);
    wrong.entities[1].identity =
        serde_json::json!({"server_id":input.server_id(),"startup":{"pid":7}}).to_string();
    assert!(
        store
            .publish_process_collector_revision_checked(
                "job-one",
                &input,
                (1, 1001),
                "wrong",
                &wrong,
                || Ok(())
            )
            .is_err()
    );
    assert_eq!(count(&store, "collector_runs"), 0);
    assert_eq!(
        store
            .connection
            .execute("DELETE FROM node_unix_observations WHERE node_id=2", [])
            .unwrap(),
        1
    );
    assert!(matches!(
        store.publish_process_collector_revision_checked(
            "job-two",
            &input,
            (1, 1001),
            "missing",
            &batch(&input, "missing", true, false),
            || Ok(())
        ),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(count(&store, "process_job_publication_receipts"), 0);
}
#[test]
fn bounded_selection_is_complete_or_refuses_without_demoting_any_run() {
    let (_, mut store, input, _) = fixture();
    store
        .publish_process_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "first",
            &batch(&input, "first", true, false),
            || Ok(()),
        )
        .unwrap();
    let mut reads = diskgraph_core::QueryReadBudget::new(
        diskgraph_core::QueryBudget {
            max_edges: 1,
            max_response_bytes: 4096,
            ..diskgraph_core::QueryBudget::default()
        },
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        store
            .process_collector_selection_with_budget("first", &mut reads)
            .unwrap(),
        vec![("first".into(), "active".into())]
    );
    let mut reads = diskgraph_core::QueryReadBudget::new(
        diskgraph_core::QueryBudget {
            max_edges: 1,
            max_response_bytes: 1,
            ..diskgraph_core::QueryBudget::default()
        },
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(
        store.process_collector_selection_with_budget("first", &mut reads),
        Err(StoreError::BudgetExceeded)
    ));
}
#[test]
fn obsolete_process_receipt_writer_and_cross_kind_job_id_are_rejected() {
    let (_, mut store, input, _) = fixture();
    let receipt = store
        .publish_process_collector_revision_checked(
            "same-job",
            &input,
            (1, 1001),
            "first",
            &batch(&input, "first", true, false),
            || Ok(()),
        )
        .unwrap();
    assert!(store.connection.execute("INSERT INTO process_job_publication_receipts VALUES('old',0,?1,?2,?3,?4,'old-revision','old-run',2,?5,13)",rusqlite::params![receipt.input_sha256(),input.server_id().as_str(),input.scope_id().as_str(),receipt.snapshot_id(),serde_json::to_string(&receipt).unwrap()]).is_err());
    assert!(store.connection.execute("INSERT INTO job_publication_receipts VALUES('same-job',1,?1,?2,?3,?4,'git-revision','git-run',2,?5,13)",rusqlite::params![receipt.input_sha256(),input.server_id().as_str(),input.scope_id().as_str(),receipt.snapshot_id(),"{}"]).is_err());
    assert_eq!(count(&store, "job_publication_receipts"), 0);
    assert_eq!(
        store.process_job_publication_receipt("same-job").unwrap(),
        Some(receipt)
    );
}

#[test]
fn dangling_selected_process_source_cannot_publish_a_complete_revision() {
    let (_, mut store, input, _) = fixture();
    store
        .publish_process_collector_revision_checked(
            "first-job",
            &input,
            (1, 1001),
            "first",
            &batch(&input, "first", true, false),
            || Ok(()),
        )
        .unwrap();
    let next_input = next(&input, "first");
    // 仅模拟外部关闭 FK 后的损坏；正常存储 API 不允许删除所选来源。
    store
        .connection
        .pragma_update(None, "foreign_keys", "OFF")
        .unwrap();
    assert_eq!(
        store
            .connection
            .execute("DELETE FROM collector_runs WHERE run_id='first'", [])
            .unwrap(),
        1
    );
    store
        .connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let flags:(i64,i64)=store.connection.query_row("SELECT selection_sealed,evidence_complete FROM graph_revisions WHERE revision_id='first'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(flags, (1, 1));
    let tables = [
        "graph_revisions",
        "revision_ownership",
        "revision_runs",
        "collector_runs",
        "entities",
        "evidence_records",
        "relations",
        "process_job_publication_receipts",
    ];
    let before = tables.map(|table| count(&store, table));
    assert!(matches!(
        store.publish_process_collector_revision_checked(
            "next-job",
            &next_input,
            (1, 1002),
            "next",
            &batch(&next_input, "next", false, true),
            || Ok(())
        ),
        Err(StoreError::InvalidGraph(_))
    ));
    let mut reads = diskgraph_core::QueryReadBudget::new(
        diskgraph_core::QueryBudget::default(),
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(
        store.process_collector_selection_with_budget("first", &mut reads),
        Err(StoreError::InvalidGraph(_))
    ));
    for (table, expected) in tables.into_iter().zip(before) {
        assert_eq!(count(&store, table), expected, "rollback of {table}");
    }
    assert_eq!(
        store.process_job_publication_receipt("next-job").unwrap(),
        None
    );
    assert!(matches!(
        store.revision("next"),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert_eq!(
        store
            .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
            .unwrap()
            .as_deref(),
        Some("first")
    );
}
