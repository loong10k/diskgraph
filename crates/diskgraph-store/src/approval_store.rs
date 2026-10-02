//! 批准的身份、动作、摘要、期限及实时撤销验证。

use crate::approval_row::ApprovalRow;
use rusqlite::OptionalExtension;

use crate::execution_codec::{action_from_name, action_name, bad_state, now_ms};
use crate::{Approval, ControlStore, Result, StoreError};
use diskgraph_core::{FileActionKind, PrincipalId};
use rusqlite::params;

impl ControlStore {
    /// Records a trusted approval for one plan digest.
    /// 持久记录可信批准，使用前由 verify_approval 校验其绑定与有效性。
    /// 参数：approval：可信批准记录。
    /// 返回：写入记录的 approval_ref；数据库写入失败返回错误。
    pub fn insert_approval(&mut self, approval: &Approval) -> Result<String> {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO approvals (approval_ref, plan_id, plan_digest, principal, action, issued_by, issued_at_unix_ms, expires_at_unix_ms, revoked)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    approval.approval_ref,
                    approval.plan_id,
                    approval.plan_digest,
                    approval.principal.as_str(),
                    action_name(approval.action),
                    approval.issued_by,
                    approval.issued_at_unix_ms as i64,
                    approval.expires_at_unix_ms as i64,
                    approval.revoked as i64,
                ],
            )?;
            Ok(approval.approval_ref.clone())
        })
    }

    /// Looks an approval up, refusing revoked, expired, or foreign-plan uses.
    /// The approval must match the exact plan id, plan digest, principal, and
    /// action requested, so a client cannot widen a narrow approval (OP-03).
    /// 持久校验精确批准的摘要、主体、动作、期限与撤销。
    /// 参数：approval_ref：批准引用；wanted_plan_id：应与批准一致的计划 ID；wanted_plan_digest：应与批准一致的精确计划摘要；wanted_principal：应与批准一致的真实主体；wanted_action：应与批准一致的动作能力。
    /// 返回：`Result<Approval>` 的当前持久查询结果；数据库/格式/状态错误向调用者传播。
    pub fn verify_approval(
        &self,
        approval_ref: &str,
        wanted_plan_id: &str,
        wanted_plan_digest: &str,
        wanted_principal: &PrincipalId,
        wanted_action: FileActionKind,
    ) -> Result<Approval> {
        let row: Option<ApprovalRow> = self.with_connection(|connection| {
                connection
                    .query_row(
                        "SELECT plan_id, plan_digest, principal, action, issued_by, issued_at_unix_ms, expires_at_unix_ms, revoked
                         FROM approvals WHERE approval_ref = ?1",
                        [approval_ref],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get(6)?,
                                row.get(7)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(StoreError::from)
            })?;
        let (plan_id, plan_digest, principal, action, issued_by, issued_at, expires_at, revoked) =
            row.ok_or_else(|| StoreError::ApprovalRequired("unknown approval".into()))?;
        if revoked != 0 {
            return Err(StoreError::ApprovalRequired("approval revoked".into()));
        }
        if plan_id != wanted_plan_id || plan_digest != wanted_plan_digest {
            return Err(StoreError::ApprovalRequired(
                "approval is bound to a different plan".into(),
            ));
        }
        if principal != wanted_principal.as_str() {
            return Err(StoreError::ApprovalRequired(
                "approval is for another principal".into(),
            ));
        }
        let action =
            action_from_name(&action).ok_or_else(|| bad_state("approvals.action", &action))?;
        if action != wanted_action {
            return Err(StoreError::ApprovalRequired(
                "approval is for another action".into(),
            ));
        }
        if now_ms() >= expires_at.max(0) as u64 {
            return Err(StoreError::ApprovalRequired("approval expired".into()));
        }
        Ok(Approval {
            approval_ref: approval_ref.to_owned(),
            plan_id,
            plan_digest,
            principal: PrincipalId::new(principal)
                .map_err(|error| StoreError::InvalidGraph(error.to_string()))?,
            action,
            issued_by,
            issued_at_unix_ms: issued_at.max(0) as u64,
            expires_at_unix_ms: expires_at.max(0) as u64,
            revoked: false,
        })
    }

    /// Revokes an approval before it is used.
    /// 持久校验精确批准的摘要、主体、动作、期限与撤销。
    /// 参数：approval_ref：批准引用。
    /// 返回：成功为 ()，数据库/格式或状态冲突以 StoreError 返回。
    pub fn revoke_approval(&mut self, approval_ref: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "UPDATE approvals SET revoked = 1 WHERE approval_ref = ?1",
                [approval_ref],
            )?;
            Ok(())
        })
    }
}
