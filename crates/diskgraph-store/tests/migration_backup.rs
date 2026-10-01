#[test]
fn migration_backup_includes_committed_wal_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let store = diskgraph_store::SqliteSnapshotStore::open(&path).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("DROP TABLE revision_ownership; PRAGMA user_version = 4; PRAGMA wal_autocheckpoint = 0; CREATE TABLE backup_probe (value TEXT); INSERT INTO backup_probe VALUES ('committed-in-wal');").unwrap();
    let (_, backup) =
        diskgraph_store::SqliteSnapshotStore::open_with_backup(&path, &dir.path().join("backups"))
            .unwrap();
    let backup = rusqlite::Connection::open(backup.unwrap()).unwrap();
    let value: String = backup
        .query_row("SELECT value FROM backup_probe", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "committed-in-wal");
    drop(store);
}

#[test]
fn v7_upgrade_backs_up_the_previous_schema_and_builds_candidate_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let store = diskgraph_store::SqliteSnapshotStore::open(&path).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute_batch(
            "DROP INDEX nodes_by_candidate_size;
         DROP INDEX evidence_by_relation_node;
         PRAGMA user_version = 7;
         PRAGMA wal_autocheckpoint = 0;
         CREATE TABLE upgrade_probe (value TEXT);
         INSERT INTO upgrade_probe VALUES ('preserved');",
        )
        .unwrap();
    let (upgraded, backup) =
        diskgraph_store::SqliteSnapshotStore::open_with_backup(&path, &dir.path().join("backups"))
            .unwrap();
    let backup = rusqlite::Connection::open(backup.unwrap()).unwrap();
    let old_version: i64 = backup
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(old_version, 7);
    let retained: String = backup
        .query_row("SELECT value FROM upgrade_probe", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "preserved");
    drop(backup);
    let upgraded_db = rusqlite::Connection::open(&path).unwrap();
    let new_version: i64 = upgraded_db
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(new_version, diskgraph_store::SUPPORTED_SCHEMA_VERSION);
    let indexes: i64 = upgraded_db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name IN ('nodes_by_candidate_size', 'evidence_by_relation_node')",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(indexes, 2);
    drop(upgraded);
    drop(store);
}
