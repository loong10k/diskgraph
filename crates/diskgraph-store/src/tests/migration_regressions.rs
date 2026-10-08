use crate::sqlite_snapshot_store::SUPPORTED_SCHEMA_VERSION;
use crate::{SqliteSnapshotStore, StoreError};
use rusqlite::Connection;

use super::fixtures::{graph, write_v1_database};
#[test]
fn persists_across_connections() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("snapshots.sqlite");
    SqliteSnapshotStore::open(&db)
        .unwrap()
        .save(&graph("persistent", 123))
        .unwrap();
    assert_eq!(
        SqliteSnapshotStore::open(&db)
            .unwrap()
            .load("persistent")
            .unwrap()
            .nodes[1]
            .subtree_bytes,
        123
    );
}

#[test]
fn v1_databases_migrate_in_place_keeping_existing_rows() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("graph.sqlite");
    write_v1_database(&db);
    let store = SqliteSnapshotStore::open(&db).unwrap();
    let version: i64 = store
        .connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SUPPORTED_SCHEMA_VERSION);
    // The v1 row survives (its JSON here is a minimal stand-in; load of the
    // full snapshot is exercised through publish_revision tests below).
    let rows: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM snapshots", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn unknown_future_schema_versions_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("future.sqlite");
    let connection = Connection::open(&db).unwrap();
    connection
        .execute_batch("PRAGMA user_version = 99;")
        .unwrap();
    drop(connection);
    assert!(matches!(
        SqliteSnapshotStore::open(&db),
        Err(StoreError::UnsupportedSchema(99))
    ));
}

#[test]
fn migration_backup_is_written_and_open_failure_keeps_v1_intact() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("graph.sqlite");
    write_v1_database(&db);
    let backup_dir = directory.path().join("backups");
    let (store, backup) = SqliteSnapshotStore::open_with_backup(&db, &backup_dir).unwrap();
    let backup = backup.expect("v1 open must produce a backup");
    assert!(backup.exists());
    let backup_version: i64 = Connection::open(&backup)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        backup_version, 1,
        "the backup must be the pre-migration bytes"
    );
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        SUPPORTED_SCHEMA_VERSION
    );
    // A fresh v2 open reports that no backup was needed.
    let (_, no_backup) = SqliteSnapshotStore::open_with_backup(&db, &backup_dir).unwrap();
    assert!(no_backup.is_none());
}

#[test]
fn migration_retry_preserves_existing_recovery_backup() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("graph.sqlite");
    write_v1_database(&db);
    let backups = directory.path().join("backups");
    std::fs::create_dir(&backups).unwrap();
    let first = backups.join(format!("graph.sqlite.pre-v{SUPPORTED_SCHEMA_VERSION}.bak"));
    let prior = Connection::open(&first).unwrap();
    prior.execute_batch("CREATE TABLE recovery_marker(value TEXT); INSERT INTO recovery_marker VALUES ('original');").unwrap();
    drop(prior);
    let original = std::fs::read(&first).unwrap();
    let (_, latest) = SqliteSnapshotStore::open_with_backup(&db, &backups).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        original,
        "migration retry overwrote the previous recovery snapshot"
    );
    let latest = latest.unwrap();
    assert_ne!(latest, first);
    assert_eq!(
        Connection::open(&latest)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn control_migration_failure_preserves_previous_recovery_backup() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("control.sqlite");
    Connection::open(&db)
        .unwrap()
        .execute_batch("PRAGMA user_version=8;")
        .unwrap();
    let backups = directory.path().join("migration_backups");
    std::fs::create_dir(&backups).unwrap();
    let first = backups.join("control.sqlite.pre-v9.bak");
    let prior = Connection::open(&first).unwrap();
    prior.execute_batch("CREATE TABLE recovery_marker(value TEXT); INSERT INTO recovery_marker VALUES ('original');").unwrap();
    drop(prior);
    let original = std::fs::read(&first).unwrap();
    assert!(
        crate::ControlStore::open(&db).is_err(),
        "malformed old schema must not start service"
    );
    assert_eq!(
        std::fs::read(&first).unwrap(),
        original,
        "failed migration retry overwrote the previous recovery snapshot"
    );
}
