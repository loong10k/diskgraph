//! Process 发布前对真实已索引普通文件的强身份复验；来源：原生 Rust D42 / EV-06。
use crate::{Result, StoreError};
use diskgraph_core::{IndexedFileEpoch, ProcessEvidenceJobInput, QueryBudget, QueryReadBudget};
use rusqlite::{Connection, params};
/// 参数：已持有发布事务、原 snapshot 与原输入；返回：同一扫描身份已证明，缺失要求重索引。
pub(crate) fn require(
    connection: &Connection,
    snapshot: &str,
    input: &ProcessEvidenceJobInput,
) -> Result<()> {
    match input.indexed_epoch() {
        IndexedFileEpoch::Windows {
            volume,
            file_id,
            creation_ticks,
        } => {
            let mut stmt=connection.prepare("SELECT native_observation_format,native_observation_raw,native_observation_gap,native_locator_kind,native_locator_encoding FROM nodes WHERE snapshot_id=?1 AND id=?2")?;
            let mut rows = stmt.query(params![snapshot, input.node_id() as i64])?;
            let row = rows
                .next()?
                .ok_or_else(|| StoreError::InvalidGraph("missing process target".into()))?;
            let fields = [
                row.get_ref(0)?,
                row.get_ref(1)?,
                row.get_ref(2)?,
                row.get_ref(3)?,
                row.get_ref(4)?,
            ];
            let bytes = fields.iter().try_fold(0_usize, |n, v| {
                n.checked_add(match v {
                    rusqlite::types::ValueRef::Null => 0,
                    rusqlite::types::ValueRef::Integer(_) | rusqlite::types::ValueRef::Real(_) => 8,
                    rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
                        b.len()
                    }
                })
                .ok_or(StoreError::BudgetExceeded)
            })?;
            if bytes > 1024 {
                return Err(StoreError::BudgetExceeded);
            }
            let stored = crate::windows_observation_codec::decode(
                fields[0], fields[1], fields[2], fields[3], fields[4],
            )?;
            let o = stored
                .observation
                .ok_or_else(|| StoreError::Conflict("unverified process target; reindex".into()))?;
            if o.volume != *volume
                || o.file_id != *file_id
                || o.creation_time != *creation_ticks
                || o.directory
                || o.delete_pending
                || o.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
            {
                return Err(StoreError::Conflict("process target epoch mismatch".into()));
            }
        }
        _ => {
            let mut reads = QueryReadBudget::new(
                QueryBudget {
                    max_nodes: 1,
                    max_response_bytes: 2048,
                    ..QueryBudget::default()
                },
                std::time::Instant::now() + std::time::Duration::from_secs(1),
            )
            .map_err(|_| StoreError::BudgetExceeded)?;
            let stored = crate::unix_observation_query::read(
                connection,
                snapshot,
                input.node_id(),
                &mut reads,
            )?
            .ok_or_else(|| StoreError::InvalidGraph("missing process target".into()))?;
            let o = stored
                .observation
                .ok_or_else(|| StoreError::Conflict("unverified process target; reindex".into()))?;
            if o.epoch() != input.indexed_epoch() {
                return Err(StoreError::Conflict("process target epoch mismatch".into()));
            }
        }
    }
    Ok(())
}
