//! 操作幂等键、对象占用、意图和不可回退的终态。

use rusqlite::OptionalExtension;

use crate::execution_codec::{bad_state, claim_keys, key_overlap, now_ms};
use crate::{
    ControlStore, IntentState, Operation, OperationItem, OperationItemResult, OperationState, Plan,
    Result, StoreError,
};
use diskgraph_core::{PrincipalId, ScopeId};
use rusqlite::params;

impl ControlStore {
    /// Creates an operation, or returns the existing one for a reused
    /// idempotency key. A different request under the same key is refused
    /// (OP-08).
    /// 建立或读取持久幂等操作和逐项记录。
    /// 参数：operation：实际操作记录；items：新建操作时初始化的条目数量。
    /// 返回：(operation_id, created)；true 表示新建，false 表示幂等复用；同一键绑定不同请求时返回冲突。
    pub fn begin_operation(
        &mut self,
        operation: &Operation,
        items: usize,
    ) -> Result<(String, bool)> {
        self.with_connection(|connection| {
            // A single immediate transaction serializes claim checks across
            // independent processes; a check before this lock can both pass
            // and let two writers touch the same source or destination.
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let result = (|| -> Result<(String, bool)> {
            let existing: Option<(String, String)> = connection
                .query_row(
                    "SELECT operation_id, request_digest FROM operations
                     WHERE principal = ?1 AND idempotency_key = ?2",
                    params![operation.principal.as_str(), operation.idempotency_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((operation_id, digest)) = existing {
                if digest != operation.request_digest {
                    return Err(StoreError::IdempotencyConflict);
                }
                return Ok((operation_id, false));
            }
            let new_json: String = connection.query_row(
                "SELECT plan_json FROM plans WHERE plan_id = ?1",
                [&operation.plan_id],
                |row| row.get(0),
            )?;
            let new_plan: Plan = serde_json::from_str(&new_json)?;
            if new_plan.scope_id != operation.scope_id || new_plan.principal != operation.principal {
                return Err(StoreError::Conflict(
                    "operation subject or scope does not match its plan".into(),
                ));
            }
            let requested = claim_keys(&new_plan);
            let mut statement = connection.prepare(
                "SELECT o.operation_id, p.plan_json FROM operations o
                 JOIN plans p ON p.plan_id = o.plan_id
                 WHERE o.state IN ('queued', 'revalidating', 'running', 'needs_attention')",
            )?;
            let active = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in active {
                let (other_id, other_json) = row?;
                let other_plan: Plan = serde_json::from_str(&other_json)?;
                let held = claim_keys(&other_plan);
                if requested.iter().any(|mine| held.iter().any(|other| key_overlap(mine, other))) {
                    return Err(StoreError::Conflict(format!(
                        "operation {other_id} already claims an overlapping source or target"
                    )));
                }
            }
            connection.execute(
                "INSERT INTO operations (operation_id, plan_id, scope_id, principal, idempotency_key, request_digest, state, created_at_unix_ms, updated_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', ?7, ?7)",
                params![
                    operation.operation_id,
                    operation.plan_id,
                    operation.scope_id.as_str(),
                    operation.principal.as_str(),
                    operation.idempotency_key,
                    operation.request_digest,
                    operation.created_at_unix_ms as i64,
                ],
            )?;
            for index in 0..items {
                connection.execute(
                    "INSERT INTO operation_items (operation_id, item_index, intent_state, result_state, detail, recovery_ref)
                     VALUES (?1, ?2, 'pending', 'pending', '', NULL)",
                    params![operation.operation_id, index as i64],
                )?;
            }
            Ok((operation.operation_id.clone(), true))
            })();
            match result {
                Ok(value) => {
                    connection.execute_batch("COMMIT")?;
                    Ok(value)
                }
                Err(error) => {
                    let _ = connection.execute_batch("ROLLBACK");
                    Err(error)
                }
            }
        })
    }

    /// The operation recorded for one principal's idempotency key, if any.
    /// 建立或读取持久幂等操作和逐项记录。
    /// 参数：principal：真实请求主体；idempotency_key：主体幂等请求键。
    /// 返回：`Result<Option<Operation>>` 可选记录，None 表示无匹配；数据库/解码失败返回错误。
    pub fn operation_for_key(
        &self,
        principal: &PrincipalId,
        idempotency_key: &str,
    ) -> Result<Option<Operation>> {
        let id: Option<String> = self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT operation_id FROM operations
                     WHERE principal = ?1 AND idempotency_key = ?2",
                    params![principal.as_str(), idempotency_key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(StoreError::from)
        })?;
        match id {
            Some(id) => Ok(Some(self.operation(&id)?)),
            None => Ok(None),
        }
    }

    /// Loads one operation.
    /// 建立或读取持久幂等操作和逐项记录。
    /// 参数：operation_id：持久操作 ID。
    /// 返回：`Result<Operation>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn operation(&self, operation_id: &str) -> Result<Operation> {
        self.with_connection(|connection| {
            let row: Option<(String, String, String, String, String, i64, i64)> = connection
                .query_row(
                    "SELECT plan_id, scope_id, principal, idempotency_key, request_digest, created_at_unix_ms, updated_at_unix_ms
                     FROM operations WHERE operation_id = ?1",
                    [operation_id],
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
            let (plan_id, scope, principal, key, digest, created, updated) = row
                .ok_or_else(|| StoreError::OperationNotFound(operation_id.to_owned()))?;
            let state: String = connection.query_row(
                "SELECT state FROM operations WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )?;
            Ok(Operation {
                operation_id: operation_id.to_owned(),
                plan_id,
                scope_id: ScopeId::new(scope).map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
                principal: PrincipalId::new(principal)
                    .map_err(|e| StoreError::InvalidGraph(e.to_string()))?,
                idempotency_key: key,
                request_digest: digest,
                state: OperationState::parse(&state).ok_or_else(|| bad_state("operations", &state))?,
                created_at_unix_ms: created.max(0) as u64,
                updated_at_unix_ms: updated.max(0) as u64,
            })
        })
    }

    /// Sets an operation's state through the trusted compatibility interface.
    /// 可信内部兼容 setter，无条件更新状态；调用方负责合法转换，执行器应使用 advance_operation_state。
    /// 参数：operation_id：持久操作 ID；state：目标生命周期状态。
    /// 返回：SQL 更新成功为 ()；不存在的 ID 不产生行，数据库失败返回错误。
    pub fn set_operation_state(&mut self, operation_id: &str, state: OperationState) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE operations SET state = ?2, updated_at_unix_ms = ?3 WHERE operation_id = ?1",
                params![operation_id, state.wire_name(), now_ms() as i64],
            )?;
            Ok(())
        })
    }

    /// 原子推进非终态操作；已取消、结束或需要恢复的状态不允许被执行器覆盖。
    /// 记录意图、实际结果和状态，终态不可回退。
    /// 参数：operation_id：持久操作 ID；state：目标生命周期状态。
    /// 返回：`Result<OperationState>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn advance_operation_state(
        &mut self,
        operation_id: &str,
        state: OperationState,
    ) -> Result<OperationState> {
        self.with_connection(|connection| {
            connection.execute("UPDATE operations SET state=?2,updated_at_unix_ms=?3 WHERE operation_id=?1 AND state IN ('queued','revalidating','running')", params![operation_id,state.wire_name(),now_ms() as i64])?;
            let value: String = connection.query_row("SELECT state FROM operations WHERE operation_id=?1", [operation_id],|row| row.get(0))?;
            OperationState::parse(&value).ok_or_else(|| bad_state("operations",&value))
        })
    }

    /// Every item of an operation, in index order.
    /// 建立或读取持久幂等操作和逐项记录。
    /// 参数：operation_id：持久操作 ID。
    /// 返回：`Result<Vec<OperationItem>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn operation_items(&self, operation_id: &str) -> Result<Vec<OperationItem>> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT item_index, intent_state, result_state, detail, recovery_ref
                 FROM operation_items WHERE operation_id = ?1 ORDER BY item_index ASC",
            )?;
            let rows = statement.query_map([operation_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
            let mut items = Vec::new();
            for row in rows {
                let (index, intent, result, detail, recovery_ref) = row?;
                items.push(OperationItem {
                    operation_id: operation_id.to_owned(),
                    item_index: index.max(0) as u32,
                    intent: IntentState::parse(&intent)
                        .ok_or_else(|| bad_state("operation_items.intent_state", &intent))?,
                    result: OperationItemResult::parse(&result)
                        .ok_or_else(|| bad_state("operation_items.result_state", &result))?,
                    detail,
                    recovery_ref,
                });
            }
            Ok(items)
        })
    }

    /// Records the intent for one item. This must happen before the file is
    /// touched (OP-08).
    /// 记录意图、实际结果和状态，终态不可回退。
    /// 参数：operation_id：持久操作 ID；index：计划条目序号，不能作节点 ID。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn record_intent(&mut self, operation_id: &str, index: u32) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE operation_items SET intent_state = 'intent_recorded'
                 WHERE operation_id = ?1 AND item_index = ?2 AND intent_state = 'pending'",
                params![operation_id, index as i64],
            )?;
            Ok(())
        })
    }

    /// Records the outcome of one item.
    /// 记录意图、实际结果和状态，终态不可回退。
    /// 参数：operation_id：持久操作 ID；index：计划条目序号，不能作节点 ID；result：实际逐项执行结果；detail：审计结果诊断；recovery_ref：恢复引用。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn record_item_result(
        &mut self,
        operation_id: &str,
        index: u32,
        result: OperationItemResult,
        detail: &str,
        recovery_ref: Option<&str>,
    ) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE operation_items
                 SET result_state = ?3, detail = ?4, recovery_ref = ?5
                 WHERE operation_id = ?1 AND item_index = ?2",
                params![
                    operation_id,
                    index as i64,
                    result.wire_name(),
                    detail,
                    recovery_ref,
                ],
            )?;
            Ok(())
        })
    }

    /// Operations for a scope, newest first (C26 listing).
    /// 建立或读取持久幂等操作和逐项记录。
    /// 参数：scope_id：实际所属范围 ID；limit：最大页条数。
    /// 返回：`Result<Vec<Operation>>` 结果集合，空集合表示无匹配，顺序遵循本查询 SQL。
    pub fn list_operations(&self, scope_id: &ScopeId, limit: u64) -> Result<Vec<Operation>> {
        let ids: Vec<String> = self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT operation_id FROM operations WHERE scope_id = ?1
                 ORDER BY created_at_unix_ms DESC, operation_id DESC LIMIT ?2",
            )?;
            let rows =
                statement.query_map(params![scope_id.as_str(), limit as i64], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })?;
        ids.iter().map(|id| self.operation(id)).collect()
    }
}
