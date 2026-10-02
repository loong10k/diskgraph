//! 不可变计划、摘要和计划状态的持久化。

use rusqlite::OptionalExtension;

use crate::execution_codec::{action_name, bad_state, now_ms};
use crate::{ControlStore, Plan, PlanState, Result, StoreError};
use rusqlite::params;

impl ControlStore {
    /// Persists an immutable plan and returns its digest.
    /// 写入、读取或条件消费不可变计划及绑定摘要。
    /// 参数：plan：精确不可变计划；digest：绑定计划的精确摘要。
    /// 返回：该记录的标识或绑定摘要，冲突或缺失以错误返回。
    pub fn insert_plan(&mut self, plan: &Plan, digest: &str) -> Result<String> {
        let json = serde_json::to_string(plan)
            .map_err(|error| StoreError::InvalidGraph(error.to_string()))?;
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO plans (plan_id, scope_id, principal, action, plan_json, digest, created_at_unix_ms, expires_at_unix_ms, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'validated')",
                params![
                    plan.plan_id,
                    plan.scope_id.as_str(),
                    plan.principal.as_str(),
                    action_name(plan.action),
                    json,
                    digest,
                    plan.created_at_unix_ms as i64,
                    plan.expires_at_unix_ms as i64,
                ],
            )?;
            Ok(plan.plan_id.clone())
        })
    }

    /// Loads a plan by id.
    /// 写入、读取或条件消费不可变计划及绑定摘要。
    /// 参数：plan_id：计划 ID。
    /// 返回：`Result<Plan>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn plan(&self, plan_id: &str) -> Result<Plan> {
        let json: String = self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT plan_json FROM plans WHERE plan_id = ?1",
                    [plan_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::PlanNotFound(plan_id.to_owned()))
        })?;
        serde_json::from_str(&json).map_err(|error| StoreError::InvalidGraph(error.to_string()))
    }

    /// The digest recorded for a plan.
    /// 写入、读取或条件消费不可变计划及绑定摘要。
    /// 参数：plan_id：计划 ID。
    /// 返回：该记录的标识或绑定摘要，冲突或缺失以错误返回。
    pub fn plan_digest(&self, plan_id: &str) -> Result<String> {
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT digest FROM plans WHERE plan_id = ?1",
                    [plan_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| StoreError::PlanNotFound(plan_id.to_owned()))
        })
    }

    /// Current plan state, or `expired` once the deadline passed.
    /// 写入、读取或条件消费不可变计划及绑定摘要。
    /// 参数：plan_id：计划 ID。
    /// 返回：`Result<PlanState>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn plan_state(&self, plan_id: &str) -> Result<PlanState> {
        let (state, expires_at, now): (String, i64, i64) = self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT state, expires_at_unix_ms, ?2 FROM plans WHERE plan_id = ?1",
                    params![plan_id, now_ms() as i64],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
                .ok_or_else(|| StoreError::PlanNotFound(plan_id.to_owned()))
        })?;
        let state = PlanState::parse(&state).ok_or_else(|| bad_state("plans", &state))?;
        if state == PlanState::Validated && now >= expires_at {
            return Ok(PlanState::Expired);
        }
        Ok(state)
    }

    /// Marks a plan consumed; a plan can be applied at most once.
    /// 写入、读取或条件消费不可变计划及绑定摘要。
    /// 参数：plan_id：计划 ID。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn mark_plan_applied(&mut self, plan_id: &str) -> Result<()> {
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE plans SET state = 'applied' WHERE plan_id = ?1 AND state = 'validated'",
                [plan_id],
            )?;
            if changed == 0 {
                return Err(StoreError::Conflict(format!(
                    "plan {plan_id} is not in a validated state"
                )));
            }
            Ok(())
        })
    }

    /// Revokes a plan before it is applied.
    /// 写入、读取或条件消费不可变计划及绑定摘要。
    /// 参数：plan_id：计划 ID。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn revoke_plan(&mut self, plan_id: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE plans SET state = 'revoked' WHERE plan_id = ?1 AND state = 'validated'",
                [plan_id],
            )?;
            Ok(())
        })
    }
}
