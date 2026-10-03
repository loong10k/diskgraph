use crate::node_codec::as_i64;
use crate::node_row::NodeRow;
use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskNode, QueryReadBudget};
use rusqlite::params;

impl SqliteSnapshotStore {
    /// 对精确节点的所有借用列先准入，再拥有字符串/解码旧 JSON。
    /// 参数：snapshot/node_id 固定节点，budget 为同请求累计原始字段/节点账本。
    /// 返回：节点或 None；超限返回 BudgetExceeded，真实列/JSON错误保留。
    pub fn node_with_budget(
        &self,
        snapshot: &str,
        node_id: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<DiskNode>> {
        let mut statement = self.connection.prepare("SELECT id,parent_id,locator_key,name,subtree_bytes,node_json,kind,direct_bytes,files,directories,modified_unix_seconds,file_volume_id,file_id,category_hint,reclaim_hint,read_error FROM nodes WHERE snapshot_id=?1 AND id=?2 LIMIT 1")?;
        let mut rows = statement.query(params![snapshot, as_i64(node_id)?])?;
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let bytes = NodeRow::raw_bytes(row)?;
        if !budget.admit(1, 0, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        Ok(Some(NodeRow::from_row(row)?.into_node()?))
    }
}
