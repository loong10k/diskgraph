//! 快照元数据、可信完整加载与窄树行读取。

use rusqlite::OptionalExtension;

use crate::{Result, SqliteSnapshotStore, StoreError, TreeRow};
use diskgraph_core::{DiskGraph, DiskSnapshot, ResourceLocator};
use serde_json::{from_str, to_string};

impl SqliteSnapshotStore {
    /// 读取指定快照、节点或窄树行，完整加载仅供可信内部使用。
    /// 参数：id：快照 ID。
    /// 返回：`Result<DiskSnapshot>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn snapshot(&self, id: &str) -> Result<DiskSnapshot> {
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT snapshot_json FROM snapshots WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        match json {
            Some(json) => {
                let confirmed: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM snapshot_counts WHERE snapshot_id=?1)",
                    [id],
                    |row| row.get(0),
                )?;
                if !confirmed {
                    return Err(StoreError::InvalidGraph(
                        "snapshot count indexes are missing; reindex this scope".into(),
                    ));
                }
                Ok(from_str(&json)?)
            }
            None => Err(StoreError::SnapshotNotFound(id.to_owned())),
        }
    }

    /// 读取指定快照、节点或窄树行，完整加载仅供可信内部使用。
    /// 参数：root：无损根定位或根过滤条件。
    /// 返回：`Result<Option<String>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn latest_snapshot_id(&self, root: &ResourceLocator) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM snapshots WHERE root_key = ?1
                 ORDER BY captured_at_unix_ms DESC, id DESC LIMIT 1",
                [to_string(root)?],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// 读取指定快照、节点或窄树行，完整加载仅供可信内部使用。
    /// 参数：id：快照 ID。
    /// 返回：`Result<DiskGraph>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn load(&self, id: &str) -> Result<DiskGraph> {
        let snapshot = self.snapshot(id)?;
        let nodes = self.load_nodes(id)?;
        let evidence = self.select_json(
            "SELECT evidence_json FROM evidence WHERE snapshot_id = ?1 ORDER BY rowid",
            id,
        )?;
        Ok(DiskGraph {
            snapshot,
            nodes,
            evidence,
        })
    }

    /// The narrow rows a tree view needs. Requires v4 structured columns; a
    /// pre-v4 snapshot yields no rows and the caller falls back to full load.
    /// 读取指定快照、节点或窄树行，完整加载仅供可信内部使用。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：`Result<Vec<TreeRow>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn tree_rows(&self, snapshot_id: &str) -> Result<Vec<TreeRow>> {
        let mut statement = self.connection.prepare(
            "SELECT id, parent_id, name, kind, subtree_bytes, direct_bytes,
                    files, directories, read_error, category_hint
             FROM nodes WHERE snapshot_id = ?1 AND kind IS NOT NULL ORDER BY id",
        )?;
        let rows = statement.query_map([snapshot_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                parent_id,
                name,
                kind,
                subtree_bytes,
                direct_bytes,
                files,
                directories,
                read_error,
                category,
            ) = row?;
            Ok((
                id as u64,
                parent_id.map(|value| value as u64),
                name,
                kind,
                subtree_bytes,
                direct_bytes,
                files,
                directories,
                read_error,
                category,
            ))
        })
        .collect()
    }
}
