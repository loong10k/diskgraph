//! 节点解码、单节点与定位查询。

use crate::full_node_row::FullNodeRow;
use rusqlite::OptionalExtension;

use crate::node_codec::{as_i64, kind_from_name};
use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::{DiskNode, ResourceLocator};
use rusqlite::params;
use serde_json::{from_str, to_string};

impl SqliteSnapshotStore {
    /// Loads every node of one snapshot. Pre-v4 rows have NULL structured
    /// columns and fall back to a full JSON parse; v4 rows construct the node
    /// directly, paying one ~50-byte locator parse instead of a ~500-byte
    /// full-node parse (measured: the JSON parse was the dominant cost of a
    /// 4.3M-row load).
    /// 读取指定快照、节点或窄树行，完整加载仅供可信内部使用。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：`Result<Vec<DiskNode>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub(crate) fn load_nodes(&self, snapshot_id: &str) -> Result<Vec<DiskNode>> {
        use rayon::prelude::{ParallelIterator, ParallelSlice};

        let mut statement = self.connection.prepare(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 ORDER BY id",
        )?;
        let rows: Vec<FullNodeRow> = statement
            .query_map([snapshot_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let chunks: Vec<Result<Vec<DiskNode>>> = rows
            .par_chunks(16_384)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|row| -> Result<DiskNode> {
                        let (
                            id,
                            parent_id,
                            locator_key,
                            name,
                            subtree_bytes,
                            node_json,
                            kind,
                            direct_bytes,
                            files,
                            directories,
                            modified_unix_seconds,
                            file_volume_id,
                            file_id,
                            category_hint,
                            reclaim_hint,
                            read_error,
                        ) = row;
                        if let Some(kind) = kind {
                            // v4 fast path: the only remaining text parse is
                            // the locator's small envelope.
                            let locator: ResourceLocator = from_str(locator_key)?;
                            let file_identity = match (file_volume_id, file_id) {
                                (Some(volume_id), Some(id)) => Some(diskgraph_core::FileIdentity {
                                    volume_id: volume_id.clone(),
                                    file_id: *id as u64,
                                }),
                                _ => None,
                            };
                            Ok(DiskNode {
                                id: *id as u64,
                                parent_id: parent_id.map(|value| value as u64),
                                locator,
                                name: name.clone(),
                                kind: kind_from_name(kind)?,
                                subtree_bytes: *subtree_bytes as u64,
                                direct_bytes: direct_bytes.unwrap_or(0) as u64,
                                files: files.unwrap_or(0) as u64,
                                directories: directories.unwrap_or(0) as u64,
                                modified_unix_seconds: *modified_unix_seconds,
                                file_identity,
                                category_hint: category_hint.clone(),
                                reclaim_hint: reclaim_hint.clone(),
                                read_error: read_error.unwrap_or(0) != 0,
                                // Structured rows are only written for fully
                                // measured nodes; unknown sizes stay in JSON.
                                size_known: true,
                            })
                        } else {
                            // Pre-v4 row: parse the archived JSON payload.
                            Ok(from_str(node_json)?)
                        }
                    })
                    .collect()
            })
            .collect();
        let mut nodes = Vec::with_capacity(rows.len());
        for chunk in chunks {
            nodes.append(&mut chunk?);
        }
        Ok(nodes)
    }

    /// The root node of one snapshot (the only node without a parent).
    /// A summary that only needs the subtree total reads this single row
    /// instead of materializing every node.
    /// 按精确定位/ID 读取节点，不加载整棵树。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：`Result<Option<DiskNode>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn root_node(&self, snapshot_id: &str) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        self.one_node(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 AND parent_id IS NULL LIMIT 1",
            params![snapshot_id],
        )
    }

    /// 按精确定位/ID 读取节点，不加载整棵树。
    /// 参数：snapshot_id：固定快照 ID；node_id：精确节点 ID。
    /// 返回：`Result<Option<DiskNode>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn node(&self, snapshot_id: &str, node_id: u64) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        self.one_node(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 AND id = ?2 LIMIT 1",
            params![snapshot_id, as_i64(node_id)?],
        )
    }

    /// Decodes one row of the node column list, or nothing when the row is
    /// absent. Shared by the single-row lookups so a measured node is read
    /// from its columns and only a pre-v4 row falls back to its payload.
    /// 按精确定位/ID 读取节点，不加载整棵树。
    /// 参数：sql：内部固定 SQL 模板；params：参数化查询绑定值。
    /// 返回：`Result<Option<DiskNode>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub(crate) fn one_node<P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
    ) -> Result<Option<DiskNode>> {
        let Some(row) = self
            .connection
            .query_row(sql, params, |row| Ok(NodeRow::from(row)))
            .optional()?
        else {
            return Ok(None);
        };
        Ok(Some(row.into_node()?))
    }

    /// The one child of `parent_id` named `name`, or nothing.
    ///
    /// Unlike `children` this asks for a single entry, so a directory with
    /// ten thousand children costs one index probe instead of a page of
    /// rows the caller would only filter down.
    /// 按精确定位/ID 读取节点，不加载整棵树。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；name：精确名称或 wire 标签。
    /// 返回：`Result<Option<DiskNode>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn child_named(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        name: &str,
    ) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        self.one_node(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes WHERE snapshot_id = ?1 AND parent_id = ?2 AND name = ?3 LIMIT 1",
            params![snapshot_id, as_i64(parent_id)?, name],
        )
    }

    /// One node by its exact locator.
    ///
    /// A compatibility lookup, not a query path: no production read uses it,
    /// and without the locator index it scans the snapshot. Anything on a
    /// hot path wants a node id and `revision_layer` instead — an id is
    /// `O(log n)` on the parent index, this is `O(n)`.
    /// 按精确定位/ID 读取节点，不加载整棵树。
    /// 参数：snapshot_id：固定快照 ID；locator：无损资源定位。
    /// 返回：`Result<Option<DiskNode>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn node_by_locator(
        &self,
        snapshot_id: &str,
        locator: &ResourceLocator,
    ) -> Result<Option<DiskNode>> {
        self.snapshot(snapshot_id)?;
        // Read the structured columns, not the archived payload: a measured
        // node no longer carries one, and this must not depend on a row's
        // age. A pre-v4 row still falls back to its payload, which is where
        // its fields live.
        self.connection
            .query_row(
                "SELECT id FROM nodes
                 WHERE snapshot_id = ?1 AND locator_key = ?2 LIMIT 1",
                params![snapshot_id, to_string(locator)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .and_then(|id| self.node(snapshot_id, id as u64).transpose())
            .transpose()
    }
}
