use crate::Result;
use rusqlite::Connection;

/// The v1 schema, verbatim: fresh databases are created at v1 and then
/// migrated forward, so v1 rows never skip the migration path (ST-02).
pub(crate) const V1_SCHEMA: &str = "PRAGMA foreign_keys = ON;
     BEGIN IMMEDIATE;
     CREATE TABLE IF NOT EXISTS snapshots (
         id TEXT PRIMARY KEY,
         root_key TEXT NOT NULL,
         captured_at_unix_ms INTEGER NOT NULL,
         snapshot_json TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS snapshots_by_root_time
         ON snapshots (root_key, captured_at_unix_ms DESC, id DESC);
     CREATE TABLE IF NOT EXISTS nodes (
         snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
         id INTEGER NOT NULL,
         parent_id INTEGER,
         locator_key TEXT NOT NULL,
         name TEXT NOT NULL,
         subtree_bytes INTEGER NOT NULL,
         node_json TEXT NOT NULL,
         PRIMARY KEY (snapshot_id, id)
     );
     CREATE INDEX IF NOT EXISTS nodes_by_parent_size
         ON nodes (snapshot_id, parent_id, subtree_bytes DESC, name ASC);
     CREATE TABLE IF NOT EXISTS evidence (
         snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
         node_id INTEGER NOT NULL,
         evidence_json TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS evidence_by_node
         ON evidence (snapshot_id, node_id);
     PRAGMA user_version = 1;
     COMMIT;";

/// Adds the v2 layer (revisions, latest pointers, staging, retention pin) in
/// one transaction; a failed migration rolls back and the v1 file stays v1.
/// 在已有事务策略下升级结构或构建精确计数，失败向调用者传播。
/// 参数：connection：调用者控制的 SQLite 连接。
/// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
pub(crate) fn migrate_v1_to_v2(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "ALTER TABLE snapshots ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
         CREATE TABLE graph_revisions (
             revision_id TEXT PRIMARY KEY,
             snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
             published_at_unix_ms INTEGER NOT NULL
         );
         CREATE INDEX revisions_by_snapshot
             ON graph_revisions (snapshot_id, published_at_unix_ms DESC);
         CREATE TABLE latest_revision (
             root_key TEXT PRIMARY KEY,
             revision_id TEXT NOT NULL REFERENCES graph_revisions(revision_id)
         );
         CREATE TABLE scan_staging (
             job_id TEXT NOT NULL,
             node_seq INTEGER NOT NULL,
             node_json TEXT NOT NULL,
             PRIMARY KEY (job_id, node_seq)
         );
         PRAGMA user_version = 2;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// Adds the evidence layer: collector runs, typed entities and relations with
/// provenance, and the revision-to-run binding (EV-01..EV-05).
/// 在已有事务策略下升级结构或构建精确计数，失败向调用者传播。
/// 参数：connection：调用者控制的 SQLite 连接。
/// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
pub(crate) fn migrate_v2_to_v3(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "CREATE TABLE collector_runs (
             run_id TEXT PRIMARY KEY,
             snapshot_id TEXT NOT NULL,
             collector_id TEXT NOT NULL,
             collector_version INTEGER NOT NULL,
             rule_version INTEGER NOT NULL,
             observed_at_unix_ms INTEGER NOT NULL,
             coverage_complete INTEGER NOT NULL,
             run_json TEXT NOT NULL
         );
         CREATE INDEX runs_by_snapshot
             ON collector_runs (snapshot_id, observed_at_unix_ms DESC);
         CREATE TABLE entities (
             snapshot_id TEXT NOT NULL,
             entity_id TEXT NOT NULL,
             kind TEXT NOT NULL,
             entity_json TEXT NOT NULL,
             PRIMARY KEY (snapshot_id, entity_id)
         );
         CREATE TABLE relations (
             snapshot_id TEXT NOT NULL,
             edge_id TEXT NOT NULL,
             source_entity_id TEXT NOT NULL,
             relation TEXT NOT NULL,
             target_entity_id TEXT NOT NULL,
             edge_json TEXT NOT NULL,
             PRIMARY KEY (snapshot_id, edge_id)
         );
         CREATE INDEX relations_by_source
             ON relations (snapshot_id, source_entity_id, relation);
         CREATE INDEX relations_by_target
             ON relations (snapshot_id, target_entity_id, relation);
         CREATE TABLE evidence_records (
             snapshot_id TEXT NOT NULL,
             evidence_id TEXT NOT NULL,
             run_id TEXT NOT NULL,
             evidence_json TEXT NOT NULL,
             PRIMARY KEY (snapshot_id, evidence_id)
         );
         CREATE INDEX evidence_by_run
             ON evidence_records (snapshot_id, run_id);
         CREATE TABLE revision_runs (
             revision_id TEXT NOT NULL REFERENCES graph_revisions(revision_id),
             run_id TEXT NOT NULL REFERENCES collector_runs(run_id),
             role TEXT NOT NULL,
             PRIMARY KEY (revision_id, run_id)
         );
         PRAGMA user_version = 3;",
    )?;
    transaction.commit()?;
    Ok(())
}

/// v3 -> v4: nodes gain structured columns mirroring node_json, so
/// multi-million-row loads stop paying a full JSON parse per row. Pre-v4
/// rows keep NULL in these columns and take the JSON fallback at read time.
/// 在已有事务策略下升级结构或构建精确计数，失败向调用者传播。
/// 参数：connection：调用者控制的 SQLite 连接。
/// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
pub(crate) fn migrate_v3_to_v4(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "ALTER TABLE nodes ADD COLUMN kind TEXT;
         ALTER TABLE nodes ADD COLUMN direct_bytes INTEGER;
         ALTER TABLE nodes ADD COLUMN files INTEGER;
         ALTER TABLE nodes ADD COLUMN directories INTEGER;
         ALTER TABLE nodes ADD COLUMN modified_unix_seconds INTEGER;
         ALTER TABLE nodes ADD COLUMN file_volume_id TEXT;
         ALTER TABLE nodes ADD COLUMN file_id INTEGER;
         ALTER TABLE nodes ADD COLUMN category_hint TEXT;
         ALTER TABLE nodes ADD COLUMN reclaim_hint TEXT;
         ALTER TABLE nodes ADD COLUMN read_error INTEGER;
         PRAGMA user_version = 4;",
    )?;
    transaction.commit()?;
    Ok(())
}
