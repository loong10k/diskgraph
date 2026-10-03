//! 历史比较的有序行迭代器。

use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::DiskNode;
use rusqlite::params;

impl SqliteSnapshotStore {
    /// 通过 SQLite 有序路径游标逐条解码节点，调用者只保留两侧当前条目。
    /// 在同一读连接内按路径顺序逐行访问历史节点。
    /// 参数：snapshot_id：固定快照 ID；root：无损根定位或根过滤条件；work：有效事务期间的内部回调。
    /// 返回：有序行迭代回调结果，不同时加载两棵完整树。
    pub fn with_ordered_nodes<T>(
        &self,
        snapshot_id: &str,
        root: &str,
        work: impl FnOnce(&mut dyn Iterator<Item = Result<(String, DiskNode)>>) -> Result<T>,
    ) -> Result<T> {
        let mut statement = self.connection.prepare(ORDERED_NODES_SQL)?;
        let mapped = statement.query_map(params![snapshot_id, root], |row| {
            Ok((row.get::<_, String>(16)?, NodeRow::from_row(row)?))
        })?;
        let mut nodes = mapped.map(|row| {
            let (path, row) = row?;
            // 比较结果使用跨平台 `/` 相对路径；Windows SQL 路径保留原生 `\`，
            // 且前导分隔符不会被 SQL 的 Unix ltrim 去掉。
            #[cfg(windows)]
            let path = path.trim_start_matches('\\').replace('\\', "/");
            Ok((path, row.into_node()?))
        });
        work(&mut nodes)
    }
}
pub(crate) const ORDERED_NODES_SQL: &str = "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json, kind, direct_bytes, files, directories, modified_unix_seconds, file_volume_id, file_id, category_hint, reclaim_hint, read_error, CASE WHEN substr(json_extract(locator_key, '$.value'), 1, length(?2)) = ?2 THEN ltrim(substr(json_extract(locator_key, '$.value'), length(?2) + 1), '/') ELSE name END AS relative_path FROM nodes WHERE snapshot_id = ?1 AND parent_id IS NOT NULL ORDER BY json_extract(locator_key, '$.value') ASC, id ASC";
