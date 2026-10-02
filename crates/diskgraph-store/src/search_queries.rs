//! Unicode 子串匹配与 name/ID keyset 搜索。

use crate::node_codec::{as_i64, read_node_page};
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::DiskNode;
use rusqlite::params;

impl SqliteSnapshotStore {
    /// Unicode 小写子串匹配，以 name/id keyset 分页；offset 仅供首次请求兼容。
    /// 按 Unicode 小写子串匹配并以 name/ID seek。
    /// 参数：snapshot_id：固定快照 ID；pattern：Unicode 小写子串条件；after：上一页真实 keyset 位置；offset：显式跳过条目数；limit：最大页条数。
    /// 返回：匹配节点页及是否仍有后续。
    pub fn search_page(
        &self,
        snapshot_id: &str,
        pattern: &str,
        after: Option<(&str, u64)>,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<DiskNode>, bool)> {
        self.snapshot(snapshot_id)?;
        let seek = if after.is_some() {
            "AND (n.name,n.id)>(?3,?4)"
        } else {
            ""
        };
        let sql = format!(
            "SELECT n.id, n.parent_id, n.locator_key, n.name, n.subtree_bytes, n.node_json, n.kind, n.direct_bytes, n.files, n.directories, n.modified_unix_seconds, n.file_volume_id, n.file_id, n.category_hint, n.reclaim_hint, n.read_error FROM nodes n JOIN node_search s ON s.snapshot_id = n.snapshot_id AND s.id = n.id WHERE n.snapshot_id = ?1 AND (instr(s.name_fold, ?2) > 0 OR instr(s.path_fold, ?2) > 0) {seek} ORDER BY n.name ASC, n.id ASC LIMIT ?5 OFFSET ?6"
        );
        let mut statement = self.connection.prepare(&sql)?;
        read_node_page(
            &mut statement,
            params![
                snapshot_id,
                pattern.to_lowercase(),
                after.map(|key| key.0),
                after.map(|key| as_i64(key.1)).transpose()?,
                as_i64(limit.saturating_add(1))?,
                as_i64(if after.is_some() { 0 } else { offset })?
            ],
            limit,
        )
    }
}
