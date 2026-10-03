use crate::{Result, SqliteSnapshotStore, StoreError};
use diskgraph_core::{DiskSnapshot, QueryReadBudget};
use rusqlite::OptionalExtension;

impl SqliteSnapshotStore {
    /// 在历史元数据分配与 JSON 解码前计入独立原始字段额度。
    /// 参数：snapshot 为固定快照，budget 为同次准备和数据阶段账本。
    /// 返回：兼容完整元数据；真实格式错误与缺少计数索引保持拒绝。
    pub fn snapshot_with_budget(
        &self,
        snapshot: &str,
        budget: &mut QueryReadBudget,
    ) -> Result<DiskSnapshot> {
        if !budget.check() {
            return Err(StoreError::BudgetExceeded);
        }
        let mut statement = self
            .connection
            .prepare("SELECT snapshot_json FROM snapshots WHERE id=?1")?;
        let mut rows = statement.query([snapshot])?;
        let row = rows
            .next()?
            .ok_or_else(|| StoreError::SnapshotNotFound(snapshot.to_owned()))?;
        let bytes = match row.get_ref(0)? {
            rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) => {
                value.len()
            }
            _ => 8,
        };
        if !budget.admit(0, 0, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let json: String = row.get(0)?;
        let confirmed = self
            .connection
            .query_row(
                "SELECT 1 FROM snapshot_counts WHERE snapshot_id=?1",
                [snapshot],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !confirmed {
            return Err(StoreError::InvalidGraph(
                "snapshot count indexes are missing; reindex this scope".into(),
            ));
        }
        Ok(serde_json::from_str(&json)?)
    }
}
