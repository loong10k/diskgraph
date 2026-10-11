//! 与旧目录过滤/offset顺序等价的共享原始字段预算页。
use crate::directory_aggregates::KNOWN_SIZE;
use crate::node_codec::{as_i64, read_node_page_with_budget};
use crate::{Result, SqliteSnapshotStore};
use diskgraph_core::{DiskNode, QueryReadBudget};
use rusqlite::{OptionalExtension, params};

impl SqliteSnapshotStore {
    /// 沿原请求账本读取已知大小过滤页，未知大小数保持兼容。
    /// 参数：snapshot/parent为固定资源，minimum为可选字节阈值，offset/limit为旧分页，budget为累计额度。
    /// 返回：节点、next_offset和精确未知数；原字段或时间不足时拒绝整页。
    pub fn children_page_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        minimum: Option<u64>,
        offset: u64,
        limit: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<(Vec<DiskNode>, Option<u64>, u64)> {
        self.filtered_children_page_with_budget(
            snapshot,
            parent,
            minimum,
            false,
            (offset, limit),
            budget,
        )
    }

    /// 沿同一账本读取未知大小页，不解码已知兄弟或页外探针。
    /// 参数：snapshot/parent为固定资源，offset/limit保持旧顺序，budget为原期限和累计额度。
    /// 返回：节点和next_offset；原字段或时间不足时拒绝整页。
    pub fn unknown_children_page_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        offset: u64,
        limit: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<(Vec<DiskNode>, Option<u64>)> {
        let (items, next, _) = self.filtered_children_page_with_budget(
            snapshot,
            parent,
            None,
            true,
            (offset, limit),
            budget,
        )?;
        Ok((items, next))
    }

    fn filtered_children_page_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        minimum: Option<u64>,
        unknown_only: bool,
        page: (u64, u64),
        budget: &mut QueryReadBudget,
    ) -> Result<(Vec<DiskNode>, Option<u64>, u64)> {
        self.snapshot_with_budget(snapshot, budget)?;
        let unknown = if unknown_only {
            0
        } else {
            self.connection.query_row("SELECT unknown_count FROM directory_counts WHERE snapshot_id=?1 AND parent_id=?2", params![snapshot, as_i64(parent)?], |row| row.get::<_, i64>(0)).optional()?.unwrap_or(0).max(0) as u64
        };
        let filter = if unknown_only {
            format!("NOT ({KNOWN_SIZE})")
        } else {
            format!("({KNOWN_SIZE})")
        };
        let minimum_clause = if unknown_only {
            ""
        } else {
            "AND subtree_bytes>=?3"
        };
        // WHERE谓词保持原部分索引条件，min_bytes不改成OR参数，以免宽目录失去范围检索。
        let sql = format!(
            "SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND {filter} {minimum_clause} ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?4 OFFSET ?5"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let (items, more) = read_node_page_with_budget(
            &mut statement,
            params![
                snapshot,
                as_i64(parent)?,
                as_i64(minimum.unwrap_or(0))?,
                as_i64(page.1.saturating_add(1))?,
                as_i64(page.0)?
            ],
            page.1,
            budget,
        )?;
        let next = more.then_some(page.0.saturating_add(items.len() as u64));
        Ok((items, next, unknown))
    }
}

#[cfg(test)]
#[path = "filtered_page_budget_tests.rs"]
mod filtered_page_budget_tests;
