//! 导航必要字段窄读；所有查询与解码复用原绝对期限和累计账本。

use crate::node_codec::as_i64;
use crate::{NavigationNode, Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::QueryReadBudget;
use rusqlite::{OptionalExtension, params};

// 测量行不读取 node_json，旧行才取必要解码输入；其余未使用大字段不进入 SQL 投影。
const COLUMNS: &str = "id,name,kind,subtree_bytes,files,directories,category_hint,read_error,CASE WHEN kind IS NULL THEN node_json ELSE NULL END";

impl SqliteSnapshotStore {
    /// 读取一个导航父节点，拥有必要字段之前计费。
    /// 参数：snapshot/node_id 为固定节点，budget 为整次读取的账本。返回：可选投影或真实错误。
    pub fn navigation_node_with_budget(
        &self,
        snapshot: &str,
        node_id: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<NavigationNode>> {
        self.navigation_snapshot_exists(snapshot, budget)?;
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let sql = format!("SELECT {COLUMNS} FROM nodes WHERE snapshot_id=?1 AND id=?2 LIMIT 1");
        let mut statement = self.connection.prepare(&sql)?;
        let mut rows = statement.query(params![snapshot, as_i64(node_id)?])?;
        let Some(row) = rows.next()? else {
            if !budget.check() {
                return Err(StoreError::BudgetExceeded);
            }
            return Ok(None);
        };
        Ok(Some(NavigationNode::from_row(row, budget)?))
    }

    /// 按原尺寸、名称和 ID 顺序读取页，保留显式 offset 与未知大小行。
    /// 参数：snapshot/parent/offset/limit 定义页面，budget 与父节点共用。返回：页及后续存在标记。
    /// 页外行只证明存在，不拥有或解码其 JSON；SQL/字段错误不会成为成功的部分结果。
    pub fn navigation_children_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        offset: u64,
        limit: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<(Vec<NavigationNode>, bool)> {
        self.navigation_snapshot_exists(snapshot, budget)?;
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let sql = format!(
            "SELECT {COLUMNS} FROM nodes WHERE snapshot_id=?1 AND parent_id=?2 ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT ?3 OFFSET ?4"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let mut rows = statement.query(params![
            snapshot,
            as_i64(parent)?,
            as_i64(limit)?,
            as_i64(offset)?
        ])?;
        let mut items = Vec::new();
        while (items.len() as u64) < limit {
            if !budget.check() {
                return Err(StoreError::BudgetExceeded);
            }
            let Some(row) = rows.next()? else {
                if !budget.check() {
                    return Err(StoreError::BudgetExceeded);
                }
                return Ok((items, false));
            };
            items.push(NavigationNode::from_row(row, budget)?);
        }
        drop(rows);
        drop(statement);
        // 单独的常量存在探针不读取页外旧 JSON；仍使用同一父索引顺序和原 deadline。
        // 显式 offset 保持兼容，深页仍有 offset 成本，不声称 keyset 性能。
        let next = offset
            .checked_add(limit)
            .ok_or(StoreError::IntegerOverflow)?;
        Ok((
            items,
            self.navigation_continuation_with_budget(snapshot, parent, next, budget)?,
        ))
    }

    /// 在同一期限只确认后续行存在，不读取或解码页外旧 payload。
    /// 参数：offset 是页面之后的位置，budget 沿用页面。返回：后续存在或真实错误。
    pub(crate) fn navigation_continuation_with_budget(
        &self,
        snapshot: &str,
        parent: u64,
        offset: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<bool> {
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let more = self.connection.query_row("SELECT 1 FROM nodes INDEXED BY nodes_by_parent_size WHERE snapshot_id=?1 AND parent_id=?2 ORDER BY subtree_bytes DESC,name ASC,id ASC LIMIT 1 OFFSET ?3", params![snapshot, as_i64(parent)?, as_i64(offset)?], |_| Ok(())).optional()?.is_some();
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(more)
    }

    fn navigation_snapshot_exists(
        &self,
        snapshot: &str,
        budget: &mut QueryReadBudget,
    ) -> Result<()> {
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM snapshots WHERE id=?1",
                [snapshot],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(StoreError::SnapshotNotFound(snapshot.to_owned()));
        }
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(())
    }
}
