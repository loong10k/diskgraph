use crate::node_row::NodeRow;
use crate::{Result, StoreError};
use diskgraph_core::{DiskNode, QueryReadBudget, TruncationReason};

/// SQLite 有序历史行的借用游标；来源：DiskGraph Q-08/D24 双侧实际读取预算。
/// 游标仅在所属 statement 生命周期内使用，不拥有完整历史或新数据库连接。
pub struct HistoryNodeCursor<'statement> {
    pub(crate) rows: rusqlite::Rows<'statement>,
}

impl HistoryNodeCursor<'_> {
    /// 准入借用字段与实际节点后才拥有路径、解码节点。
    /// 参数：budget 为双侧及准备阶段共享账本。返回：当前节点、EOF 或存储/预算错误。
    /// 节点额度为零时下一行仅证明存在，不接触其定位/JSON，也不吞容量内格式错误。
    pub fn next(&mut self, budget: &mut QueryReadBudget) -> Result<Option<(String, DiskNode)>> {
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let Some(row) = self.rows.next()? else {
            return Ok(None);
        };
        if budget.remaining_nodes() == 0 {
            budget.stop(TruncationReason::NodeLimit);
            return Err(StoreError::BudgetExceeded);
        }
        let path_bytes = match row.get_ref(16)? {
            rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) => {
                value.len()
            }
            _ => 8,
        };
        let bytes = NodeRow::raw_bytes(row)?
            .checked_add(path_bytes)
            .ok_or(StoreError::BudgetExceeded)?;
        if !budget.admit(1, 0, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let path = row.get::<_, String>(16)?;
        #[cfg(windows)]
        let path = path.trim_start_matches('\\').replace('\\', "/");
        Ok(Some((path, NodeRow::from_row(row)?.into_node()?)))
    }
}
