use crate::git_job_test_fixtures::fixture;
use crate::{ControlStore, JobKind, SqliteSnapshotStore, StoreError};
use rusqlite::Connection;

fn version(connection: &Connection) -> i64 {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}
fn remove_control_v8(connection: &Connection) {
    connection.execute_batch("DROP TRIGGER IF EXISTS git_input_rejects_process; DROP TABLE IF EXISTS process_job_failures; DROP TABLE IF EXISTS process_evidence_job_inputs; DROP TRIGGER git_job_input_immutable_update; DROP TRIGGER git_job_input_immutable_delete;
        DROP TRIGGER git_job_failure_no_update; DROP TRIGGER git_job_failure_no_delete;
        DROP TABLE git_job_failures; DROP TABLE git_evidence_job_inputs; PRAGMA user_version=7;").unwrap();
}
fn remove_graph_v13(connection: &Connection) {
    connection
        .execute_batch(
            "DROP VIEW IF EXISTS revision_authorized_ownership; DROP TABLE IF EXISTS revision_access_denials; DROP TRIGGER IF EXISTS git_receipt_no_process_job; DROP TABLE IF EXISTS process_job_publication_receipts; DROP TABLE IF EXISTS node_unix_observations; DROP TABLE IF EXISTS scan_staging_unix_observations; DROP TRIGGER job_receipt_no_update; DROP TRIGGER job_receipt_no_delete;
        DROP TABLE job_publication_receipts; PRAGMA user_version=12;",
        )
        .unwrap();
}

#[test]
fn v7_control_upgrade_backs_up_exact_wal_state_and_never_steals_a_live_running_lease() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_job_with_authority(input.scope_id(), JobKind::Index, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    control
        .connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .unwrap();
    let old = Connection::open(&path).unwrap();
    remove_control_v8(&old);
    old.pragma_update(None, "journal_mode", "WAL").unwrap();
    old.execute(
        "UPDATE jobs SET heartbeat_unix_ms=heartbeat_unix_ms+1 WHERE job_id=?1",
        [&job.job_id],
    )
    .unwrap();
    let upgraded = ControlStore::open(&path).unwrap();
    assert_eq!(version(&upgraded.connection), 9);
    let actual = upgraded.job(&job.job_id).unwrap();
    assert_eq!(actual.owner, running.owner);
    assert_eq!(actual.fencing_token, running.fencing_token);
    assert_eq!(actual.lease_expires_unix_ms, running.lease_expires_unix_ms);
    assert_eq!(actual.heartbeat_unix_ms, running.heartbeat_unix_ms + 1);
    assert_eq!(
        upgraded.job_request_authority(&job.job_id).unwrap(),
        Some(authority)
    );
    let backup = Connection::open(
        directory
            .path()
            .join("migration_backups/control.sqlite.pre-v8.bak"),
    )
    .unwrap();
    assert_eq!(version(&backup), 7);
    assert_eq!(
        backup
            .query_row(
                "SELECT heartbeat_unix_ms FROM jobs WHERE job_id=?1",
                [&job.job_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap() as u64,
        actual.heartbeat_unix_ms
    );
    assert!(
        backup
            .prepare("SELECT * FROM git_evidence_job_inputs")
            .is_err()
    );
}

#[test]
fn failed_v8_control_migration_keeps_the_old_version_and_consistent_backup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    {
        let store = ControlStore::open(&path).unwrap();
        remove_control_v8(&store.connection);
        store
            .connection
            .execute_batch(
                "CREATE TABLE git_evidence_job_inputs(job_id TEXT PRIMARY KEY,wrong TEXT);",
            )
            .unwrap();
    }
    assert!(matches!(
        ControlStore::open(&path),
        Err(StoreError::InvalidGraph(_))
    ));
    for path in [
        &path,
        &directory
            .path()
            .join("migration_backups/control.sqlite.pre-v8.bak"),
    ] {
        let connection = Connection::open(path).unwrap();
        assert_eq!(version(&connection), 7);
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='git_job_failures' OR name LIKE 'git_job_input_immutable_%'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}

#[test]
fn graph_v12_upgrade_has_an_exact_backup_and_preserves_the_existing_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let (_, graph, input, _) = fixture();
    graph
        .connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .unwrap();
    {
        let old = Connection::open(&path).unwrap();
        remove_graph_v13(&old);
    }
    let (upgraded, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")).unwrap();
    assert_eq!(version(&upgraded.connection), 15);
    assert_eq!(
        upgraded
            .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
            .unwrap()
            .as_deref(),
        Some("base")
    );
    let backup = Connection::open(backup.unwrap()).unwrap();
    assert_eq!(version(&backup), 12);
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM graph_revisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        backup
            .prepare("SELECT * FROM job_publication_receipts")
            .is_err()
    );
}

#[test]
fn failed_graph_v13_migration_does_not_enable_a_partially_created_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    {
        let store = SqliteSnapshotStore::open(&path).unwrap();
        remove_graph_v13(&store.connection);
        store
            .connection
            .execute_batch(
                "CREATE TABLE job_publication_receipts(job_id TEXT PRIMARY KEY,wrong TEXT);",
            )
            .unwrap();
    }
    assert!(
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")).is_err()
    );
    for path in [
        &path,
        &directory.path().join("backups/graph.sqlite.pre-v15.bak"),
    ] {
        let connection = Connection::open(path).unwrap();
        assert_eq!(version(&connection), 12);
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'job_receipt_no_%'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}
