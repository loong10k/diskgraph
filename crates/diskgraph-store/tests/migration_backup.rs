#[test]
fn migration_backup_includes_committed_wal_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let store = diskgraph_store::SqliteSnapshotStore::open(&path).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("DROP TRIGGER revisions_require_collector_writer; DROP TRIGGER collectors_require_member_writer;
         DROP TRIGGER revisions_preserve_seal; DROP TRIGGER selected_runs_no_append;
         DROP TRIGGER selected_runs_no_rewrite; DROP TRIGGER selected_runs_no_remove;
         ALTER TABLE graph_revisions DROP COLUMN writer_generation;
         ALTER TABLE graph_revisions DROP COLUMN selection_sealed;
         ALTER TABLE graph_revisions DROP COLUMN evidence_complete;
         ALTER TABLE collector_runs DROP COLUMN writer_generation;
         DROP TRIGGER relation_membership_retarget; DROP TABLE relation_membership_adjacency; DROP TABLE relation_run_memberships; DROP TABLE entity_run_memberships; DROP TABLE collector_membership_diagnostics;
         DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema; DROP TABLE child_size_prefix; DROP TABLE directory_counts; DROP TABLE snapshot_counts; DROP INDEX nodes_by_known_parent_size; DROP TABLE revision_ownership;
         DROP TABLE node_search; DROP TABLE scan_staging_search; DROP INDEX nodes_by_name;
         DROP INDEX relations_by_source_edge; DROP INDEX relations_by_target_edge;
         DROP INDEX nodes_by_locator_path; DROP INDEX nodes_by_unknown_parent;
         DROP INDEX nodes_by_candidate_size; DROP INDEX evidence_by_relation_node;
         DROP INDEX nodes_by_parent_size;
         CREATE INDEX nodes_by_parent_size ON nodes(snapshot_id,parent_id,subtree_bytes DESC,name ASC);
         PRAGMA user_version = 4; PRAGMA wal_autocheckpoint = 0; CREATE TABLE backup_probe (value TEXT); INSERT INTO backup_probe VALUES ('committed-in-wal');").unwrap();
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
            "DROP TRIGGER revisions_require_collector_writer; DROP TRIGGER collectors_require_member_writer;
         DROP TRIGGER revisions_preserve_seal; DROP TRIGGER selected_runs_no_append;
         DROP TRIGGER selected_runs_no_rewrite; DROP TRIGGER selected_runs_no_remove;
         ALTER TABLE graph_revisions DROP COLUMN writer_generation;
         ALTER TABLE graph_revisions DROP COLUMN selection_sealed;
         ALTER TABLE graph_revisions DROP COLUMN evidence_complete;
         ALTER TABLE collector_runs DROP COLUMN writer_generation;
         DROP TRIGGER relation_membership_retarget; DROP TABLE relation_membership_adjacency; DROP TABLE relation_run_memberships; DROP TABLE entity_run_memberships; DROP TABLE collector_membership_diagnostics;
         DROP TRIGGER snapshots_require_count_writer; ALTER TABLE snapshots DROP COLUMN count_schema; DROP TABLE child_size_prefix;
         DROP TABLE directory_counts;
         DROP TABLE snapshot_counts;
         DROP INDEX nodes_by_known_parent_size;
         DROP INDEX nodes_by_candidate_size;
         DROP INDEX evidence_by_relation_node;
         DROP INDEX nodes_by_parent_size;
         CREATE INDEX nodes_by_parent_size ON nodes(snapshot_id,parent_id,subtree_bytes DESC,name ASC);
         DROP INDEX nodes_by_unknown_parent;
         CREATE INDEX nodes_by_unknown_parent ON nodes(snapshot_id,parent_id)
             WHERE NOT (COALESCE(read_error,json_extract(NULLIF(node_json,''),'$.read_error'),0)=0
                        AND COALESCE(json_extract(NULLIF(node_json,''),'$.size_known'),1)=1);
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
