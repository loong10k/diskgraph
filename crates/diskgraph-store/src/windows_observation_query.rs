//! 按节点主键借用准入后解码 Windows 观测，不扫描其他节点。

use crate::{Result, SqliteSnapshotStore, StoreError, StoredWindowsObservation};
use diskgraph_core::QueryReadBudget;
use rusqlite::{params, types::ValueRef};

impl SqliteSnapshotStore {
    /// 有界读取完整 Windows 原生观测，允许跨宿主读取纯元数据。
    /// 参数：snapshot_id/node_id 为固定节点键，budget 为累计节点、字节和绝对期限预算。
    /// 返回：节点不存在为 None，历史未捕获为 NotCaptured；损坏或超预算明确失败。
    pub fn windows_observation_bounded(
        &self,
        snapshot_id: &str,
        node_id: u64,
        budget: &mut QueryReadBudget,
    ) -> Result<Option<StoredWindowsObservation>> {
        check(budget)?;
        let mut statement = self.connection.prepare("SELECT native_observation_format,native_observation_raw,native_observation_gap,native_locator_kind,native_locator_encoding FROM nodes WHERE snapshot_id=?1 AND id=?2")?;
        let mut rows =
            statement.query(params![snapshot_id, crate::node_codec::as_i64(node_id)?])?;
        let Some(row) = rows.next()? else {
            check(budget)?;
            return Ok(None);
        };
        let fields = [
            row.get_ref(0)?,
            row.get_ref(1)?,
            row.get_ref(2)?,
            row.get_ref(3)?,
            row.get_ref(4)?,
        ];
        let bytes = fields.iter().try_fold(0usize, |size, field| {
            let len = match field {
                ValueRef::Null => 0,
                ValueRef::Integer(_) | ValueRef::Real(_) => 8,
                ValueRef::Text(value) | ValueRef::Blob(value) => value.len(),
            };
            size.checked_add(len).ok_or(StoreError::BudgetExceeded)
        })?;
        if !budget.admit(1, 0, bytes) {
            return Err(StoreError::BudgetExceeded);
        }
        let value = crate::windows_observation_codec::decode(
            fields[0], fields[1], fields[2], fields[3], fields[4],
        )?;
        check(budget)?;
        Ok(Some(value))
    }
}
fn check(budget: &mut QueryReadBudget) -> Result<()> {
    if budget.check() {
        Ok(())
    } else {
        Err(StoreError::BudgetExceeded)
    }
}
