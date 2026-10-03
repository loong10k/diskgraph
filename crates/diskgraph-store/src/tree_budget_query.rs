use crate::directory_aggregates::KNOWN_SIZE;
use crate::node_codec::as_i64;
use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskNode, QueryReadBudget};
use rusqlite::params;

impl SqliteSnapshotStore {
    /// 用一个有界页读取树子节点，已知/未知共用原数字阈值并分别索引。
    /// 参数：snapshot/parent/minimum 为固定查询，limit 为最多解码条数，budget 为累计账本。
    /// 返回：页及更多标记；lookahead 仅存在探针，错误与额度原因不吞掉。
    pub fn tree_children_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        minimum: u64,
        limit: usize,
        budget: &mut QueryReadBudget,
    ) -> Result<(Vec<DiskNode>, bool)> {
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let count = limit
            .checked_add(1)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(StoreError::IntegerOverflow)?;
        // 已知/未知分别匹配 v9 partial index，先取有界 ID 窗口，再统一排序/解码。
        // 不用 OR 扫过宽目录中被 minimum 排除的已知前缀；SQLite 临时状态仍不承诺 RSS。
        let sql = format!(
            "WITH known AS (SELECT id,subtree_bytes,name FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND ({KNOWN_SIZE}) AND subtree_bytes>=?3 ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?4), unknown AS (SELECT id,subtree_bytes,name FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 AND NOT ({KNOWN_SIZE}) AND subtree_bytes>=?3 ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?4), page AS (SELECT * FROM known UNION ALL SELECT * FROM unknown ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?4) SELECT n.id,n.parent_id,n.locator_key,n.name,n.subtree_bytes,n.node_json,n.kind,n.direct_bytes,n.files,n.directories,n.modified_unix_seconds,n.file_volume_id,n.file_id,n.category_hint,n.reclaim_hint,n.read_error FROM page p JOIN nodes n ON n.snapshot_id=?1 AND n.id=p.id ORDER BY p.subtree_bytes DESC,p.name ASC,p.id ASC"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let mut rows =
            statement.query(params![snapshot, as_i64(parent)?, as_i64(minimum)?, count])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            if result.len() == limit {
                return Ok((result, true));
            }
            if !budget.admit(1, 0, NodeRow::raw_bytes(row)?) {
                return Ok((result, true));
            }
            result.push(NodeRow::from_row(row)?.into_node()?);
        }
        Ok((result, false))
    }

    /// 读取树既有数字阈值的精确计数；未知语义由节点 size_known 单独保留。
    /// 参数：snapshot/parent/minimum 为固定查询。返回：总条数与可展示条数。
    pub fn tree_child_counts(
        &self,
        snapshot: &str,
        parent: u64,
        minimum: u64,
    ) -> Result<(u64, u64)> {
        self.child_counts(snapshot, parent, minimum)
    }
}
