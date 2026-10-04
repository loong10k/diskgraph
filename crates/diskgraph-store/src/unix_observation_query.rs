//! 主键窄读 Unix 强身份旁表；来源：原生 Rust D42 / FS-02。
use crate::{Result, SqliteSnapshotStore, StoreError, StoredUnixObservation};
use diskgraph_core::{QueryReadBudget, UnixFileObservation, UnixObservationGap};
use rusqlite::{Connection, params, types::ValueRef};
impl SqliteSnapshotStore {
    /// 参数：固定 snapshot/node 与原始共享账本；返回：节点缺失 None、旧缺记录 NotCaptured 或严格观察。
    /// 先借用准入原始列，再拥有有限记录；不读取源文件。
    pub fn unix_observation_bounded(
        &self,
        snapshot: &str,
        node: u64,
        reads: &mut QueryReadBudget,
    ) -> Result<Option<StoredUnixObservation>> {
        read(&self.connection, snapshot, node, reads)
    }
}
/// 参数：同一连接、实际节点与共享账本；返回：借用准入后的有限持久观察。
pub(crate) fn read(
    connection: &Connection,
    snapshot: &str,
    node: u64,
    reads: &mut QueryReadBudget,
) -> Result<Option<StoredUnixObservation>> {
    if !reads.check() {
        return Err(StoreError::BudgetExceeded);
    }
    let mut statement=connection.prepare("SELECT u.observation_raw,u.gap,u.writer_generation,n.native_locator_kind,n.native_locator_encoding FROM nodes n LEFT JOIN node_unix_observations u ON u.snapshot_id=n.snapshot_id AND u.node_id=n.id WHERE n.snapshot_id=?1 AND n.id=?2")?;
    let mut rows = statement.query(params![snapshot, crate::node_codec::as_i64(node)?])?;
    let Some(row) = rows.next()? else {
        if !reads.check() {
            return Err(StoreError::BudgetExceeded);
        }
        return Ok(None);
    };
    let fields = [
        row.get_ref(0)?,
        row.get_ref(1)?,
        row.get_ref(2)?,
        row.get_ref(3)?,
        row.get_ref(4)?,
    ];
    let bytes = fields.iter().try_fold(0_usize, |n, v| {
        n.checked_add(match v {
            ValueRef::Null => 0,
            ValueRef::Integer(_) | ValueRef::Real(_) => 8,
            ValueRef::Text(b) | ValueRef::Blob(b) => b.len(),
        })
        .ok_or(StoreError::BudgetExceeded)
    })?;
    if !reads.admit(1, 0, bytes) {
        return Err(StoreError::BudgetExceeded);
    }
    let invalid = || StoreError::InvalidGraph("invalid Unix observation columns".into());
    let value = match (fields[0], fields[1], fields[2]) {
        (ValueRef::Null, ValueRef::Null, ValueRef::Null) => StoredUnixObservation {
            observation: None,
            gap: Some(UnixObservationGap::NotCaptured),
        },
        (ValueRef::Null, ValueRef::Text(code), ValueRef::Integer(14)) => {
            let code = std::str::from_utf8(code).map_err(|_| invalid())?;
            let gap = serde_json::from_value::<UnixObservationGap>(serde_json::Value::String(
                code.into(),
            ))
            .map_err(|_| invalid())?;
            StoredUnixObservation {
                observation: None,
                gap: Some(gap),
            }
        }
        (ValueRef::Blob(raw), ValueRef::Null, ValueRef::Integer(14)) => {
            if fields[3] != ValueRef::Text(b"native_path")
                || fields[4] != ValueRef::Text(b"unix_bytes")
            {
                return Err(invalid());
            }
            StoredUnixObservation {
                observation: Some(UnixFileObservation::decode(raw).map_err(|_| invalid())?),
                gap: None,
            }
        }
        _ => return Err(invalid()),
    };
    if !reads.check() {
        return Err(StoreError::BudgetExceeded);
    }
    Ok(Some(value))
}
