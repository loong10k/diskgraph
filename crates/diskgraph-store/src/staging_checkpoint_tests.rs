use crate::{PreparedStagingNode, SqliteSnapshotStore};

#[test]
fn deferred_staging_leaves_real_committed_pages_out_of_main_until_maintenance() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("graph.sqlite");
    let copied = dir.path().join("main-only.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    store.checkpoint_after_publication().unwrap();
    store
        .connection
        .pragma_update(None, "wal_autocheckpoint", 1)
        .unwrap();
    let graph = crate::tests::graph("staging-checkpoint", 100);
    let prepared: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| PreparedStagingNode::new(node, None, None, None, None).unwrap())
        .collect();
    let mut due = false;
    store
        .append_prepared_staging_iter_checked_deferred(
            "stage",
            prepared.iter(),
            &mut due,
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        store.staging_node_count("stage").unwrap(),
        prepared.len() as u64
    );
    assert_eq!(threshold(&store), 1);
    std::fs::copy(&database, &copied).unwrap();
    let main = rusqlite::Connection::open(&copied).unwrap();
    assert_eq!(
        staged(&main),
        0,
        "automatic maintenance ran inside the fenced append"
    );
    assert!(
        due,
        "actual committed frames crossed the original threshold"
    );
    drop(main);
    store.checkpoint_after_staging().unwrap();
    std::fs::copy(&database, &copied).unwrap();
    let maintained = rusqlite::Connection::open(&copied).unwrap();
    assert_eq!(staged(&maintained), prepared.len() as i64);
}

fn threshold(store: &SqliteSnapshotStore) -> i64 {
    store
        .connection
        .query_row("PRAGMA wal_autocheckpoint", [], |r| r.get(0))
        .unwrap()
}

fn staged(connection: &rusqlite::Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM scan_staging", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn disabled_or_unreached_original_threshold_does_not_request_maintenance() {
    for original in [0, 1_000_000] {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SqliteSnapshotStore::open(&dir.path().join("graph.sqlite")).unwrap();
        store.checkpoint_after_publication().unwrap();
        store
            .connection
            .pragma_update(None, "wal_autocheckpoint", original)
            .unwrap();
        let graph = crate::tests::graph("below-threshold", 100);
        let prepared = PreparedStagingNode::new(&graph.nodes[0], None, None, None, None).unwrap();
        let mut due = false;
        store
            .append_prepared_staging_iter_checked_deferred(
                "stage",
                std::iter::once(&prepared),
                &mut due,
                || Ok(()),
            )
            .unwrap();
        assert!(!due);
        assert_eq!(threshold(&store), original);
        // 另一笔没有写入的事务也不能清掉先前已经产生的维护责任。
        due = true;
        store
            .append_prepared_staging_iter_checked_deferred(
                "empty",
                std::iter::empty(),
                &mut due,
                || Ok(()),
            )
            .unwrap();
        assert!(due);
    }
}

#[test]
fn rejected_and_panicking_staging_restore_configuration_and_roll_back() {
    for should_panic in [false, true] {
        let mut store = SqliteSnapshotStore::open_in_memory().unwrap();
        store
            .connection
            .pragma_update(None, "wal_autocheckpoint", 17)
            .unwrap();
        let graph = crate::tests::graph("rollback", 100);
        let prepared = PreparedStagingNode::new(&graph.nodes[0], None, None, None, None).unwrap();
        let mut calls = 0;
        let mut due = false;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.append_prepared_staging_iter_checked_deferred(
                "stage",
                std::iter::once(&prepared),
                &mut due,
                || {
                    calls += 1;
                    if calls < 5 {
                        return Ok(());
                    }
                    if should_panic {
                        panic!("actual staging cancellation panic");
                    }
                    Err(crate::StoreError::BudgetExceeded)
                },
            )
        }));
        if should_panic {
            assert!(result.is_err());
        } else {
            assert!(matches!(
                result.unwrap(),
                Err(crate::StoreError::BudgetExceeded)
            ));
        }
        assert_eq!(threshold(&store), 17);
        assert_eq!(staged(&store.connection), 0);
        assert!(!due);
    }
}

#[test]
fn ordinary_append_after_deferred_commit_retains_original_automatic_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("graph.sqlite");
    let copied = dir.path().join("main-only.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    store.checkpoint_after_publication().unwrap();
    store
        .connection
        .pragma_update(None, "wal_autocheckpoint", 1)
        .unwrap();
    let graph = crate::tests::graph("compatibility", 100);
    let prepared = PreparedStagingNode::new(&graph.nodes[0], None, None, None, None).unwrap();
    let mut due = false;
    store
        .append_prepared_staging_iter_checked_deferred(
            "deferred",
            std::iter::once(&prepared),
            &mut due,
            || Ok(()),
        )
        .unwrap();
    assert!(due);
    store
        .append_prepared_staging_iter_checked("ordinary", std::iter::once(&prepared), || Ok(()))
        .unwrap();
    std::fs::copy(&database, &copied).unwrap();
    assert_eq!(staged(&rusqlite::Connection::open(&copied).unwrap()), 2);
}

#[test]
fn committed_maintenance_responsibility_survives_configuration_restore_failure() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let dir = tempfile::tempdir().unwrap();
    let mut store = SqliteSnapshotStore::open(&dir.path().join("graph.sqlite")).unwrap();
    store.checkpoint_after_publication().unwrap();
    store
        .connection
        .pragma_update(None, "wal_autocheckpoint", 1)
        .unwrap();
    store
        .connection
        .authorizer(Some(|context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Pragma {
                    pragma_name: "wal_autocheckpoint",
                    pragma_value: Some("1"),
                }
            ) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    let graph = crate::tests::graph("restore-failure", 100);
    let prepared = PreparedStagingNode::new(&graph.nodes[0], None, None, None, None).unwrap();
    let mut due = false;
    let result = store.append_prepared_staging_iter_checked_deferred(
        "committed",
        std::iter::once(&prepared),
        &mut due,
        || Ok(()),
    );
    assert!(result.is_err());
    assert!(due, "committed pages lost their maintenance responsibility");
    assert_eq!(staged(&store.connection), 1);
    store
        .connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    // 恢复 SQL 被拒绝后回调必须已拆除；后续真实 commit 不得调用已释放的 Box。
    store
        .connection
        .execute("DELETE FROM scan_staging WHERE job_id='committed'", [])
        .unwrap();
    store.checkpoint_after_staging().unwrap();
    assert_eq!(staged(&store.connection), 0);
}

#[test]
fn unix_observation_commit_also_defers_actual_checkpoint() {
    use diskgraph_core::{
        LocatorEncoding, LocatorKind, NodeKind, QualifiedLocator, UnixFileObservation,
    };
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("graph.sqlite");
    let copied = dir.path().join("main-only.sqlite");
    let mut store = SqliteSnapshotStore::open(&database).unwrap();
    let mut graph = crate::tests::graph("unix-side-table", 100);
    graph.nodes[1].kind = NodeKind::File;
    graph.nodes[1].directories = 0;
    let locator = QualifiedLocator::from_parts(
        LocatorKind::NativePath,
        LocatorEncoding::UnixBytes,
        b"/tmp/diskgraph-test/cache".to_vec(),
        "/tmp/diskgraph-test/cache".into(),
    )
    .unwrap();
    store
        .append_staging_located_iter(
            "unix-stage",
            std::iter::once((&graph.nodes[1], &locator, Some(1))),
        )
        .unwrap();
    store.checkpoint_after_publication().unwrap();
    store
        .connection
        .pragma_update(None, "wal_autocheckpoint", 1)
        .unwrap();
    let observation = UnixFileObservation::new(
        crate::process_job_test_fixtures::epoch(),
        0o100600,
        100,
        (1, 2),
        (3, 4),
        (10, 11),
    )
    .unwrap();
    let mut due = false;
    store
        .append_staging_unix_observations_checked_deferred(
            "unix-stage",
            std::iter::once((2, Some(&observation), None)),
            &mut due,
            || Ok(()),
        )
        .unwrap();
    assert!(due);
    assert_eq!(threshold(&store), 1);
    std::fs::copy(&database, &copied).unwrap();
    let count = |path: &std::path::Path| {
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM scan_staging_unix_observations",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(count(&copied), 0);
    store.checkpoint_after_staging().unwrap();
    std::fs::copy(&database, &copied).unwrap();
    assert_eq!(count(&copied), 1);
}
