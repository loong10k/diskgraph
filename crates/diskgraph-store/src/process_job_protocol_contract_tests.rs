//! D42 process 持久协议的真实既有 API RED；来源：原生 Rust EC-02 / EV-05。
//! 表存在只证明迁移入口，完整发布、恢复与原生观察仍由后续行为验收覆盖。
use crate::git_job_test_fixtures::{batch, fixture};
use crate::{ControlStore, JobKind, SqliteSnapshotStore, StoreError};
use diskgraph_core::Permission;
use rusqlite::Connection;

fn version(connection: &Connection) -> i64 {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn remove_control_v9(connection: &Connection) {
    connection
        .execute_batch(
            "DROP TRIGGER IF EXISTS process_job_input_immutable_update;
             DROP TRIGGER IF EXISTS process_job_input_immutable_delete;
             DROP TRIGGER IF EXISTS process_job_failure_no_update;
             DROP TRIGGER IF EXISTS process_job_failure_no_delete;
             DROP TRIGGER IF EXISTS git_input_rejects_process;
             DROP TABLE IF EXISTS process_job_failures;
             DROP TABLE IF EXISTS process_evidence_job_inputs;
             PRAGMA user_version=8;",
        )
        .unwrap();
}

fn remove_graph_v14(connection: &Connection) {
    connection
        .execute_batch(
            "DROP VIEW IF EXISTS revision_authorized_ownership; DROP TABLE IF EXISTS revision_access_denials; DROP TRIGGER IF EXISTS process_receipt_no_update;
             DROP TRIGGER IF EXISTS process_receipt_no_delete;
             DROP TRIGGER IF EXISTS process_receipt_no_git_job;
             DROP TRIGGER IF EXISTS git_receipt_no_process_job;
             DROP TABLE IF EXISTS process_job_publication_receipts;
             DROP TABLE IF EXISTS node_unix_observations;
             DROP TABLE IF EXISTS scan_staging_unix_observations;
             PRAGMA user_version=13;",
        )
        .unwrap();
}

fn git_input_bytes(connection: &Connection, job: &str) -> (String, String, String) {
    connection
        .query_row(
            "SELECT i.input_json,i.input_sha256,a.authority_json
             FROM git_evidence_job_inputs i JOIN job_request_authorities a ON a.job_id=i.job_id
             WHERE i.job_id=?1",
            [job],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn git_receipt_bytes(connection: &Connection) -> (String, String) {
    connection
        .query_row(
            "SELECT receipt_json,input_sha256 FROM job_publication_receipts WHERE job_id='job-one'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[test]
fn durable_process_kind_has_only_metadata_and_index_permissions() {
    let kind: JobKind = serde_json::from_str("\"process_evidence\"")
        .expect("process evidence must have its own durable kind");
    assert_eq!(
        serde_json::to_string(&kind).unwrap(),
        "\"process_evidence\""
    );
    assert_eq!(
        kind.required_permissions(),
        &[Permission::MetadataRead, Permission::IndexWrite]
    );
}

#[test]
fn control_v8_upgrade_preserves_git_bytes_and_a_live_lease_in_its_backup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    let (mut original, _, input, authority) = fixture();
    let job = original
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = original
        .claim_job_once_strict(&job.job_id, "owner")
        .unwrap();
    let exact_bytes = git_input_bytes(&original.connection, &job.job_id);
    original
        .connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .unwrap();
    {
        let old = Connection::open(&path).unwrap();
        remove_control_v9(&old);
        old.pragma_update(None, "journal_mode", "WAL").unwrap();
        old.execute(
            "UPDATE jobs SET heartbeat_unix_ms=heartbeat_unix_ms+1 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    }
    let upgraded = ControlStore::open(&path).unwrap();
    assert_eq!(version(&upgraded.connection), 9);
    assert_eq!(
        git_input_bytes(&upgraded.connection, &job.job_id),
        exact_bytes
    );
    let actual = upgraded.job(&job.job_id).unwrap();
    assert_eq!(actual.owner, running.owner);
    assert_eq!(actual.fencing_token, running.fencing_token);
    assert_eq!(actual.lease_expires_unix_ms, running.lease_expires_unix_ms);
    assert_eq!(actual.heartbeat_unix_ms, running.heartbeat_unix_ms + 1);
    assert!(
        upgraded
            .connection
            .prepare("SELECT job_id,schema_version,input_json,input_sha256 FROM process_evidence_job_inputs")
            .is_ok()
    );
    assert!(
        upgraded
            .connection
            .prepare("SELECT job_id,phase,code FROM process_job_failures")
            .is_ok()
    );
    let backup = Connection::open(
        directory
            .path()
            .join("migration_backups/control.sqlite.pre-v9.bak"),
    )
    .unwrap();
    assert_eq!(version(&backup), 8);
    assert_eq!(git_input_bytes(&backup, &job.job_id), exact_bytes);
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
            .prepare("SELECT * FROM process_evidence_job_inputs")
            .is_err()
    );
}

#[test]
fn graph_v13_upgrade_preserves_git_receipt_bytes_and_the_old_input_api() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    let (_, mut original, input, _) = fixture();
    let receipt = original
        .publish_git_collector_revision_checked(
            "job-one",
            &input,
            (1, 1001),
            "git-published",
            &batch(&input, "run-one"),
            || Ok(()),
        )
        .unwrap();
    let exact_bytes = git_receipt_bytes(&original.connection);
    original
        .connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .unwrap();
    {
        let old = Connection::open(&path).unwrap();
        remove_graph_v14(&old);
    }
    let (upgraded, backup) =
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")).unwrap();
    assert_eq!(version(&upgraded.connection), 16);
    assert_eq!(git_receipt_bytes(&upgraded.connection), exact_bytes);
    assert_eq!(
        upgraded.job_publication_receipt("job-one").unwrap(),
        Some(receipt)
    );
    assert_eq!(
        upgraded
            .latest_revision_for_scope(input.server_id().as_str(), input.scope_id().as_str())
            .unwrap()
            .as_deref(),
        Some("git-published")
    );
    assert!(upgraded.connection.prepare("SELECT job_id,schema_version,input_sha256,server_id,scope_id,snapshot_id,revision_id,run_id,node_id,receipt_json,writer_generation FROM process_job_publication_receipts").is_ok());
    let backup = Connection::open(backup.unwrap()).unwrap();
    assert_eq!(version(&backup), 13);
    assert_eq!(git_receipt_bytes(&backup), exact_bytes);
    assert!(
        backup
            .prepare("SELECT * FROM process_job_publication_receipts")
            .is_err()
    );
}

#[test]
fn process_receipts_have_a_separate_current_schema() {
    let store = SqliteSnapshotStore::open_in_memory().unwrap();
    assert!(
        store
            .connection
            .prepare(
                "SELECT job_id,input_sha256,receipt_json FROM process_job_publication_receipts"
            )
            .is_ok(),
        "process receipts must not change the concrete Git v1 table or decoder"
    );
}

#[test]
fn wrong_process_input_table_refuses_control_upgrade_without_partial_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    {
        let store = ControlStore::open(&path).unwrap();
        remove_control_v9(&store.connection);
        store
            .connection
            .execute_batch(
                "CREATE TABLE process_evidence_job_inputs(job_id TEXT PRIMARY KEY,wrong TEXT);",
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
            .join("migration_backups/control.sqlite.pre-v9.bak"),
    ] {
        let old = Connection::open(path).unwrap();
        assert_eq!(version(&old), 8);
        assert_eq!(old.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='process_job_failures' OR name LIKE 'process_job_input_immutable_%'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
}

#[test]
fn wrong_process_receipt_table_refuses_graph_upgrade_without_partial_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    {
        let store = SqliteSnapshotStore::open(&path).unwrap();
        remove_graph_v14(&store.connection);
        store.connection.execute_batch("CREATE TABLE process_job_publication_receipts(job_id TEXT PRIMARY KEY,wrong TEXT);").unwrap();
    }
    assert!(matches!(
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")),
        Err(StoreError::InvalidGraph(_))
    ));
    for path in [
        &path,
        &directory.path().join("backups/graph.sqlite.pre-v16.bak"),
    ] {
        let old = Connection::open(path).unwrap();
        assert_eq!(version(&old), 13);
        assert_eq!(old.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'process_receipt_no_%' OR name='git_receipt_no_process_job'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
}

#[test]
fn same_named_unix_side_columns_with_wrong_types_cannot_enable_v14() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graph.sqlite");
    {
        let store = SqliteSnapshotStore::open(&path).unwrap();
        remove_graph_v14(&store.connection);
        store.connection.execute_batch("CREATE TABLE node_unix_observations(snapshot_id TEXT,node_id TEXT,observation_raw TEXT,gap TEXT,writer_generation TEXT,PRIMARY KEY(snapshot_id,node_id));").unwrap();
    }
    assert!(matches!(
        SqliteSnapshotStore::open_with_backup(&path, &directory.path().join("backups")),
        Err(StoreError::InvalidGraph(_))
    ));
    for path in [
        &path,
        &directory.path().join("backups/graph.sqlite.pre-v16.bak"),
    ] {
        let old = Connection::open(path).unwrap();
        assert_eq!(version(&old), 13);
        assert_eq!(old.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='process_job_publication_receipts' OR name='scan_staging_unix_observations'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    }
}
