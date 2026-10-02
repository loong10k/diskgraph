//! 目录分页、未知大小和精确聚合查询。

use crate::directory_aggregates;
use rusqlite::OptionalExtension;

use crate::node_codec::{as_i64, read_node_page};
use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::DiskNode;
use rusqlite::params;

impl SqliteSnapshotStore {
    /// Largest immediate children; use `offset` for deterministic paging.
    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；offset：显式跳过条目数；limit：最大页条数。
    /// 返回：`Result<Vec<DiskNode>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn children(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<DiskNode>> {
        self.snapshot(snapshot_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json,
                    kind, direct_bytes, files, directories, modified_unix_seconds,
                    file_volume_id, file_id, category_hint, reclaim_hint, read_error
             FROM nodes
             WHERE snapshot_id = ?1 AND parent_id = ?2
             ORDER BY subtree_bytes DESC, name ASC, id ASC LIMIT ?3 OFFSET ?4",
        )?;
        let rows = statement.query_map(
            params![
                snapshot_id,
                as_i64(parent_id)?,
                as_i64(limit)?,
                as_i64(offset)?,
            ],
            |row| Ok(NodeRow::from(row)),
        )?;
        rows.map(|row| row?.into_node()).collect()
    }

    /// 仅解码当前页节点；未知大小单独计数，保持原有列表语义。
    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；minimum：min_bytes 过滤的可选字节阈值；offset：显式跳过条目数；limit：最大页条数。
    /// 返回：节点页、可选 next_offset 和精确未知大小数。
    pub fn children_page(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        minimum: Option<u64>,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<DiskNode>, Option<u64>, u64)> {
        self.snapshot(snapshot_id)?;
        let known = directory_aggregates::KNOWN_SIZE;
        let unknown: i64 = self.connection.query_row("SELECT unknown_count FROM directory_counts WHERE snapshot_id = ?1 AND parent_id = ?2", params![snapshot_id, as_i64(parent_id)?], |row| row.get(0)).optional()?.unwrap_or(0);
        let sql = format!(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json, kind, direct_bytes, files, directories, modified_unix_seconds, file_volume_id, file_id, category_hint, reclaim_hint, read_error FROM nodes WHERE snapshot_id = ?1 AND parent_id = ?2 AND ({known}) AND subtree_bytes >= ?3 ORDER BY subtree_bytes DESC, name ASC, id ASC LIMIT ?4 OFFSET ?5"
        );
        let mut stmt = self.connection.prepare(&sql)?;
        let (items, more) = read_node_page(
            &mut stmt,
            params![
                snapshot_id,
                as_i64(parent_id)?,
                as_i64(minimum.unwrap_or(0))?,
                as_i64(limit.saturating_add(1))?,
                as_i64(offset)?
            ],
            limit,
        )?;
        let next = more.then_some(offset.saturating_add(items.len() as u64));
        Ok((items, next, unknown.max(0) as u64))
    }

    /// 只解码未知大小的当前页；与完整图的 UnknownOnly 过滤保持相同排序。
    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；offset：显式跳过条目数；limit：最大页条数。
    /// 返回：未知大小节点页及可选 next_offset。
    pub fn unknown_children_page(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<DiskNode>, Option<u64>)> {
        self.snapshot(snapshot_id)?;
        let known = directory_aggregates::KNOWN_SIZE;
        let sql = format!(
            "SELECT id, parent_id, locator_key, name, subtree_bytes, node_json, kind, direct_bytes, files, directories, modified_unix_seconds, file_volume_id, file_id, category_hint, reclaim_hint, read_error FROM nodes WHERE snapshot_id = ?1 AND parent_id = ?2 AND NOT ({known}) ORDER BY subtree_bytes DESC, name ASC, id ASC LIMIT ?3 OFFSET ?4"
        );
        let mut stmt = self.connection.prepare(&sql)?;
        let (items, more) = read_node_page(
            &mut stmt,
            params![
                snapshot_id,
                as_i64(parent_id)?,
                as_i64(limit.saturating_add(1))?,
                as_i64(offset)?
            ],
            limit,
        )?;
        let next = more.then_some(offset.saturating_add(items.len() as u64));
        Ok((items, next))
    }

    /// 有界目录 keyset 读取。参数 after 为上页最后的尺寸/名称/ID；首次保留 offset。
    /// 返回节点、是否有后续页及未知大小总数，最多解码 limit 个节点及一行存在探针。
    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；minimum：min_bytes 过滤的可选字节阈值；after：上一页真实 keyset 位置；offset：显式跳过条目数；limit：最大页条数。
    /// 返回：节点页、是否仍有后续和精确未知大小数。
    pub fn children_keyset_page(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        minimum: Option<u64>,
        after: Option<(u64, &str, u64)>,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<DiskNode>, bool, u64)> {
        let Some((last_bytes, last_name, last_id)) = after else {
            let (items, next, unknown) =
                self.children_page(snapshot_id, parent_id, minimum, offset, limit)?;
            return Ok((items, next.is_some(), unknown));
        };
        self.snapshot(snapshot_id)?;
        let unknown = self
            .connection
            .query_row(
                "SELECT unknown_count FROM directory_counts WHERE snapshot_id=?1 AND parent_id=?2",
                params![snapshot_id, as_i64(parent_id)?],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        let known = directory_aggregates::KNOWN_SIZE;
        let columns = "id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error";
        // 分别 seek 同尺寸的 name/id 和较小尺寸；每支先 LIMIT，合并最多两页。
        // 不能以带 OR 的全序条件让 SQLite 从目录开头重新过滤所有前置节点。
        let sql = format!(
            "SELECT {columns} FROM (
                SELECT * FROM (SELECT {columns} FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND ({known}) AND subtree_bytes>=?3 AND subtree_bytes=?4 AND (name,id)>(?5,?6) ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?7)
                UNION ALL
                SELECT * FROM (SELECT {columns} FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND ({known}) AND subtree_bytes>=?3 AND subtree_bytes<?4 ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?7)
             ) ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?7"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let (items, more) = read_node_page(
            &mut statement,
            params![
                snapshot_id,
                as_i64(parent_id)?,
                as_i64(minimum.unwrap_or(0))?,
                as_i64(last_bytes)?,
                last_name,
                as_i64(last_id)?,
                as_i64(limit.saturating_add(1))?
            ],
            limit,
        )?;
        Ok((items, more, unknown.max(0) as u64))
    }

    /// 精确目录总数与任意尺寸阈值计数，各做一次索引探针，不遍历子项。
    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；minimum：min_bytes 过滤的可选字节阈值。
    /// 返回：父目录总数及 minimum 过滤后的精确数量。
    pub fn child_counts(
        &self,
        snapshot_id: &str,
        parent_id: u64,
        minimum: u64,
    ) -> Result<(u64, u64)> {
        self.node_count(snapshot_id)?;
        let all: i64 = self
            .connection
            .query_row(
                "SELECT child_count FROM directory_counts WHERE snapshot_id=?1 AND parent_id=?2",
                params![snapshot_id, as_i64(parent_id)?],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let kept: i64 = self.connection.query_row("SELECT cumulative_count FROM child_size_prefix WHERE snapshot_id=?1 AND parent_id=?2 AND subtree_bytes>=?3 ORDER BY subtree_bytes ASC LIMIT 1", params![snapshot_id, as_i64(parent_id)?, as_i64(minimum)?], |row| row.get(0)).optional()?.unwrap_or(0);
        Ok((all.max(0) as u64, kept.max(0) as u64))
    }

    /// 精确节点总数，统计不要求加载节点内容。
    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID。
    /// 返回：已发布快照节点计数；未建快照为 0，存在快照但计数索引缺失报错。
    pub fn node_count(&self, snapshot_id: &str) -> Result<u64> {
        let count = self
            .connection
            .query_row(
                "SELECT node_count FROM snapshot_counts WHERE snapshot_id = ?1",
                [snapshot_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match count {
            Some(count) => Ok(count.max(0) as u64),
            None => match self.snapshot(snapshot_id) {
                Err(StoreError::SnapshotNotFound(_)) => Ok(0),
                Err(error) => Err(error),
                Ok(_) => Err(StoreError::InvalidGraph(
                    "snapshot count indexes are missing; reindex this scope".into(),
                )),
            },
        }
    }

    /// 按固定顺序与预算读取目录页或精确聚合计数。
    /// 参数：snapshot_id：固定快照 ID；parent_id：精确父节点 ID；limit：最大页条数。
    /// 返回：`Result<Vec<DiskNode>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn top(&self, snapshot_id: &str, parent_id: u64, limit: u64) -> Result<Vec<DiskNode>> {
        self.children(snapshot_id, parent_id, 0, limit)
    }
}
