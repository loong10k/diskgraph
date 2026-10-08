//! 扫描提交事实账本的不可变性与跨历史生命周期回归；生产恢复接线单独验收。
use crate::SqliteSnapshotStore;

#[test]
fn scan_receipt_storage_is_immutable_and_independent_of_history() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.connection.execute(
        "INSERT INTO scan_publication_receipts(job_id,revision_id,publishing_fence,receipt_json) VALUES ('job','revision',1,'{}')", []
    ).expect("committed scan facts need a persistent receipt ledger");
    assert!(
        store
            .connection
            .execute(
                "UPDATE scan_publication_receipts SET receipt_json='[]' WHERE job_id='job'",
                []
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "DELETE FROM scan_publication_receipts WHERE job_id='job'",
                []
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "INSERT INTO scan_publication_receipts VALUES ('job','other',2,'{}')",
                []
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "INSERT INTO scan_publication_receipts VALUES ('other','revision',2,'{}')",
                []
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "INSERT INTO scan_publication_receipts VALUES ('bad','bad',0,'{}')",
                []
            )
            .is_err()
    );
    // 回执故意不引用 snapshots/revisions 外键，历史清理不使已提交 job 可被再次发布。
    let foreign_keys: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('scan_publication_receipts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(foreign_keys, 0);
    let count: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM scan_publication_receipts WHERE job_id='job'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn scan_receipt_migration_rolls_back_on_a_conflicting_old_table() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.connection.execute_batch("DROP TRIGGER scan_receipt_no_update; DROP TRIGGER scan_receipt_no_delete; DROP TABLE scan_publication_receipts; CREATE TABLE scan_publication_receipts(wrong TEXT); PRAGMA user_version=15;").unwrap();
    assert!(crate::scan_receipt_migration::migrate(&store.connection).is_err());
    let version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 15);
    let triggers: i64 = store.connection.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name IN ('scan_receipt_no_update','scan_receipt_no_delete')", [], |r| r.get(0)).unwrap();
    assert_eq!(
        triggers, 0,
        "failed migration must not leave a partial protocol"
    );
}

#[test]
fn scan_receipt_protocol_rejects_a_replaced_immutability_trigger() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    store.connection.execute_batch("DROP TRIGGER scan_receipt_no_delete; CREATE TRIGGER scan_receipt_no_delete BEFORE DELETE ON scan_publication_receipts BEGIN SELECT 1; END;").unwrap();
    assert!(crate::scan_receipt_migration::validate(&store.connection).is_err());
    assert!(SqliteSnapshotStore::initialize(store.connection).is_err());
}

#[test]
fn v15_upgrade_preserves_history_and_adds_receipt_protocol() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = crate::tests::graph("pre-receipt-history", 100);
    store
        .publish_revision("legacy-job", &graph, "legacy-revision", 1)
        .unwrap();
    store.connection.execute_batch("DROP TRIGGER scan_receipt_no_update; DROP TRIGGER scan_receipt_no_delete; DROP TABLE scan_publication_receipts; DROP TRIGGER snapshots_require_scan_receipt_writer; ALTER TABLE snapshots DROP COLUMN scan_receipt_writer; PRAGMA user_version=15;").unwrap();
    let reopened = SqliteSnapshotStore::initialize(store.connection).unwrap();
    assert_eq!(reopened.load_revision("legacy-revision").unwrap(), graph);
    crate::scan_receipt_migration::validate(&reopened.connection).unwrap();
    let version: i64 = reopened
        .connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 16);
}

#[test]
fn corrupt_scan_receipt_is_an_error_instead_of_absence() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    assert!(store.scan_publication_receipt("absent").unwrap().is_none());
    store
        .connection
        .execute(
            "INSERT INTO scan_publication_receipts VALUES('corrupt','revision',1,'{}')",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.scan_publication_receipt("corrupt"),
        Err(crate::StoreError::InvalidGraph(_))
    ));
}

fn receipt(graph: &diskgraph_core::DiskGraph, revision: &str) -> crate::ScanPublicationReceipt {
    crate::ScanPublicationReceipt {
        job_id: "atomic-job".into(),
        kind: crate::JobKind::Index,
        principal: diskgraph_core::PrincipalId::new("actor").unwrap(),
        server_id: diskgraph_core::ServerId::new("server").unwrap(),
        scope_id: diskgraph_core::ScopeId::new("scope").unwrap(),
        authority: None,
        publishing_fence: 1,
        snapshot_id: graph.snapshot.id.clone(),
        revision_id: revision.into(),
        published_at_unix_ms: 1,
    }
}

#[test]
fn final_scan_check_rolls_back_receipt_and_revision_together() {
    let graph = crate::tests::graph("atomic-snapshot", 100);
    let receipt = receipt(&graph, "atomic-revision");
    let staging = "atomic-job:1";
    // 先测量同一发布路径的检查次数，再只在最后一次检查拒绝，覆盖回执已写入后的回滚。
    let mut successful = SqliteSnapshotStore::open_in_memory().unwrap();
    successful
        .append_staging_nodes(staging, &graph.nodes)
        .unwrap();
    let mut calls = 0;
    successful
        .publish_scan_revision_checked(staging, &graph, &receipt, None, || {
            calls += 1;
            Ok(())
        })
        .unwrap();
    assert!(calls > 1);
    assert_eq!(
        successful.scan_publication_receipt("atomic-job").unwrap(),
        Some(receipt.clone())
    );

    let mut rejected = SqliteSnapshotStore::open_in_memory().unwrap();
    rejected
        .append_staging_nodes(staging, &graph.nodes)
        .unwrap();
    let mut attempt = 0;
    let result = rejected.publish_scan_revision_checked(staging, &graph, &receipt, None, || {
        attempt += 1;
        if attempt == calls {
            Err(crate::StoreError::StaleOwner)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(crate::StoreError::StaleOwner)));
    assert_eq!(attempt, calls);
    assert!(
        rejected
            .scan_publication_receipt("atomic-job")
            .unwrap()
            .is_none()
    );
    assert!(rejected.load_revision("atomic-revision").is_err());
    assert!(rejected.snapshot("atomic-snapshot").is_err());
}

#[test]
fn duplicate_scan_job_rolls_back_second_publication() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let first = crate::tests::graph("first-snapshot", 100);
    let original = receipt(&first, "first-revision");
    store
        .append_staging_nodes("atomic-job:1", &first.nodes)
        .unwrap();
    store
        .publish_scan_revision_checked("atomic-job:1", &first, &original, None, || Ok(()))
        .unwrap();
    let second = crate::tests::graph("second-snapshot", 200);
    let mut duplicate = receipt(&second, "second-revision");
    duplicate.publishing_fence = 2;
    store
        .append_staging_nodes("atomic-job:2", &second.nodes)
        .unwrap();
    assert!(
        store
            .publish_scan_revision_checked("atomic-job:2", &second, &duplicate, None, || Ok(()))
            .is_err()
    );
    assert_eq!(
        store.scan_publication_receipt("atomic-job").unwrap(),
        Some(original)
    );
    assert_eq!(store.load_revision("first-revision").unwrap(), first);
    assert!(store.load_revision("second-revision").is_err());
    assert!(store.snapshot("second-snapshot").is_err());
}

#[test]
fn obsolete_snapshot_insert_cannot_bypass_the_new_writer_generation() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let graph = crate::tests::graph("current-writer", 100);
    store
        .publish_revision("trusted", &graph, "current-revision", 1)
        .unwrap();
    // 模拟升级前已打开连接继续执行旧版 INSERT；不能依赖它重新读取 user_version。
    let error = store.connection.execute("INSERT INTO snapshots(id,root_key,captured_at_unix_ms,snapshot_json,pinned,count_schema) SELECT 'obsolete-writer',root_key,captured_at_unix_ms,snapshot_json,pinned,count_schema FROM snapshots LIMIT 1", []).unwrap_err();
    assert!(error.to_string().contains("writer is obsolete"));
    assert_eq!(store.load_revision("current-revision").unwrap(), graph);
}

#[test]
fn history_pruning_preserves_scan_publication_receipt() {
    let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
    let first = crate::tests::graph("prunable-snapshot", 100);
    let original = receipt(&first, "prunable-revision");
    store
        .append_staging_nodes("atomic-job:1", &first.nodes)
        .unwrap();
    store
        .publish_scan_revision_checked("atomic-job:1", &first, &original, None, || Ok(()))
        .unwrap();
    let later = crate::tests::graph("retained-snapshot", 200);
    store.append_staging_nodes("later", &later.nodes).unwrap();
    store
        .publish_revision_owned(
            "later",
            &later,
            "retained-revision",
            2,
            Some(("server", "scope")),
        )
        .unwrap();
    let deleted = store.prune_revisions("server", "scope", 1, true).unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].revision_id, "prunable-revision");
    assert!(store.load_revision("prunable-revision").is_err());
    assert_eq!(
        store.scan_publication_receipt("atomic-job").unwrap(),
        Some(original)
    );
}
