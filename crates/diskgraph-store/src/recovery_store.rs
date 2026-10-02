//! 恢复记录持久化与单次消费。

use rusqlite::OptionalExtension;

use crate::execution_codec::bad_state;
use crate::{ControlStore, RecoveryEntry, RecoveryState, Result, StoreError};
use diskgraph_core::ScopeId;
use rusqlite::params;

impl ControlStore {
    /// Records a durable recovery mapping for a quarantined object.
    /// 持久保存、查询或单次消费恢复记录。
    /// 参数：entry：完整恢复记录。
    /// 返回：该记录的标识或绑定摘要，冲突或缺失以错误返回。
    pub fn insert_recovery(&mut self, entry: &RecoveryEntry) -> Result<String> {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO recovery_entries (recovery_ref, operation_id, scope_id, original_locator, quarantine_locator, identity, created_at_unix_ms, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.recovery_ref,
                    entry.operation_id,
                    entry.scope_id.as_str(),
                    entry.original_locator,
                    entry.quarantine_locator,
                    entry.identity,
                    entry.created_at_unix_ms as i64,
                    entry.state.wire_name(),
                ],
            )?;
            Ok(entry.recovery_ref.clone())
        })
    }

    /// Loads one recovery entry.
    /// 持久保存、查询或单次消费恢复记录。
    /// 参数：recovery_ref：恢复引用。
    /// 返回：`Result<RecoveryEntry>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn recovery(&self, recovery_ref: &str) -> Result<RecoveryEntry> {
        self.with_connection(|connection| {
            let row: Option<(String, String, String, String, String, i64, String)> = connection
                .query_row(
                    "SELECT operation_id, scope_id, original_locator, quarantine_locator, identity, created_at_unix_ms, state
                     FROM recovery_entries WHERE recovery_ref = ?1",
                    [recovery_ref],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let (operation_id, scope, original, quarantine, identity, created, state) = row
                .ok_or_else(|| StoreError::RecoveryNotFound(recovery_ref.to_owned()))?;
            Ok(RecoveryEntry {
                recovery_ref: recovery_ref.to_owned(),
                operation_id,
                scope_id: ScopeId::new(scope).map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
                original_locator: original,
                quarantine_locator: quarantine,
                identity,
                created_at_unix_ms: created.max(0) as u64,
                state: RecoveryState::parse(&state)
                    .ok_or_else(|| bad_state("recovery_entries.state", &state))?,
            })
        })
    }

    /// Marks a recovery entry as consumed by a restore.
    /// 持久保存、查询或单次消费恢复记录。
    /// 参数：recovery_ref：恢复引用。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn mark_recovery_restored(&mut self, recovery_ref: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE recovery_entries SET state = 'restored' WHERE recovery_ref = ?1",
                [recovery_ref],
            )?;
            Ok(())
        })
    }
}
