//! 不可变目录的精确计数索引；聚合成本只在发布/迁移事务承担。
use rusqlite::{Connection, params_from_iter};

use crate::Result;

pub(crate) const KNOWN_SIZE: &str = "COALESCE(read_error, json_extract(NULLIF(node_json, ''), '$.read_error'), 0) = 0 AND COALESCE(json_extract(NULLIF(node_json, ''), '$.size_known'), 1) = 1";

/// 在当前调用者事务中生成计数；snapshot 为 None 时回填所有旧快照。
pub(crate) fn rebuild(connection: &Connection, snapshot: Option<&str>) -> Result<()> {
    let filter = if snapshot.is_some() {
        "snapshot_id = ?1"
    } else {
        "1 = 1"
    };
    // 过滤条件只由内部固定 SQL 选择，快照 ID 始终以参数绑定。
    for table in ["child_size_prefix", "directory_counts", "snapshot_counts"] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE {filter}"),
            params_from_iter(snapshot),
        )?;
    }
    connection.execute(&format!(
        "INSERT INTO snapshot_counts SELECT s.id, COUNT(n.id) FROM snapshots s LEFT JOIN nodes n ON n.snapshot_id=s.id WHERE {} GROUP BY s.id", if snapshot.is_some() { "s.id=?1" } else { "1=1" }
    ), params_from_iter(snapshot))?;
    connection.execute(&format!(
        "INSERT INTO directory_counts
         SELECT snapshot_id, parent_id, COUNT(*), SUM(CASE WHEN NOT ({KNOWN_SIZE}) THEN 1 ELSE 0 END)
         FROM nodes WHERE {filter} AND parent_id IS NOT NULL GROUP BY snapshot_id, parent_id"
    ), params_from_iter(snapshot))?;
    // 按不同大小聚合后计算降序累计条数；任意阈值只需一次有序索引探针。
    connection.execute(&format!(
        "INSERT INTO child_size_prefix
         SELECT snapshot_id,parent_id,subtree_bytes,
                SUM(COUNT(*)) OVER (PARTITION BY snapshot_id,parent_id ORDER BY subtree_bytes DESC ROWS UNBOUNDED PRECEDING)
         FROM nodes WHERE {filter} AND parent_id IS NOT NULL
         GROUP BY snapshot_id,parent_id,subtree_bytes"
    ), params_from_iter(snapshot))?;
    Ok(())
}

/// v8→v9 同一事务创建索引并回填；失败时计数结构与版本一起回滚。
pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "ALTER TABLE snapshots ADD COLUMN count_schema INTEGER NOT NULL DEFAULT 0;
         CREATE TABLE snapshot_counts (
             snapshot_id TEXT PRIMARY KEY REFERENCES snapshots(id) ON DELETE CASCADE,
             node_count INTEGER NOT NULL CHECK(node_count >= 0)
         ) WITHOUT ROWID;
         CREATE TABLE directory_counts (
             snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
             parent_id INTEGER NOT NULL,
             child_count INTEGER NOT NULL CHECK(child_count >= 0),
             unknown_count INTEGER NOT NULL CHECK(unknown_count BETWEEN 0 AND child_count),
             PRIMARY KEY(snapshot_id,parent_id)
         ) WITHOUT ROWID;
         CREATE TABLE child_size_prefix (
             snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
             parent_id INTEGER NOT NULL,
             subtree_bytes INTEGER NOT NULL,
             cumulative_count INTEGER NOT NULL CHECK(cumulative_count > 0),
             PRIMARY KEY(snapshot_id,parent_id,subtree_bytes)
         ) WITHOUT ROWID;
         DROP INDEX nodes_by_parent_size;
         CREATE INDEX nodes_by_parent_size ON nodes(snapshot_id,parent_id,subtree_bytes DESC,name ASC,id ASC);
         DROP INDEX nodes_by_unknown_parent;",
    )?;
    transaction.execute_batch(&format!(
        "CREATE INDEX nodes_by_known_parent_size ON nodes(snapshot_id,parent_id,subtree_bytes DESC,name ASC,id ASC) WHERE {KNOWN_SIZE};
         CREATE INDEX nodes_by_unknown_parent ON nodes(snapshot_id,parent_id,subtree_bytes DESC,name ASC,id ASC) WHERE NOT ({KNOWN_SIZE});"
    ))?;
    rebuild(&transaction, None)?;
    transaction.execute_batch(
        "UPDATE snapshots SET count_schema=9;
         CREATE TRIGGER snapshots_require_count_writer BEFORE INSERT ON snapshots
         WHEN NEW.count_schema != 9
         BEGIN SELECT RAISE(ABORT, 'snapshot writer is obsolete; reopen with current DiskGraph'); END;
         PRAGMA user_version = 9;"
    )?;
    transaction.commit()?;
    Ok(())
}
