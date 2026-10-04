//! 认领和续租沿用同一控制事务；有持久身份的任务不能借可信旧入口降权。

use crate::{ControlStore, JobRecord, Result, StoreError};
use diskgraph_core::Permission;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl ControlStore {
    /// 在同一控制事务认领。参数：真实任务、owner、兼容幂等与严格来源标志。
    /// 返回：唯一代次或拒绝；只有 queued/过期 running 的授权拒绝可结算失败。
    pub(crate) fn claim_job_authorized(
        &mut self,
        job_id: &str,
        owner: &str,
        trusted_idempotent: bool,
        strict: bool,
    ) -> Result<JobRecord> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Self::now_ms();
        let state: Option<String> = tx
            .query_row("SELECT state FROM jobs WHERE job_id=?1", [job_id], |row| {
                row.get(0)
            })
            .optional()?;
        let state = state.ok_or_else(|| StoreError::JobNotFound(job_id.into()))?;
        let eligible: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE job_id=?1 AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?2)))", params![job_id,now as i64], |row| row.get(0))?;
        if !eligible {
            let idempotent: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE job_id=?1 AND state='running' AND owner=?2 AND lease_expires_unix_ms>?3)", params![job_id,owner,now as i64], |row| row.get(0))?;
            if trusted_idempotent && idempotent {
                crate::job_authority_gate::validate_job(
                    &tx,
                    job_id,
                    &[Permission::IndexWrite],
                    strict,
                )?;
                tx.commit()?;
                return self.job(job_id);
            }
            return if state == "running" {
                Err(StoreError::StaleOwner)
            } else {
                Err(StoreError::Conflict("job is not claimable".into()))
            };
        }
        if let Err(error) =
            crate::job_authority_gate::validate_job(&tx, job_id, &[Permission::IndexWrite], strict)
        {
            if matches!(error, StoreError::Conflict(_)) {
                // 只结算 queued/已过期 running；活 owner 无论 strict 与否都不能被抢占。
                tx.execute("UPDATE jobs SET state=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE 'failed' END,heartbeat_unix_ms=?2 WHERE job_id=?1 AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?2))", params![job_id,now as i64])?;
                tx.commit()?;
            }
            return Err(error);
        }
        let changed = tx.execute("UPDATE jobs SET state='running',owner=?2,heartbeat_unix_ms=?3,lease_expires_unix_ms=?4,fencing_token=fencing_token+1 WHERE job_id=?1 AND cancel_requested=0 AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?3)) AND EXISTS(SELECT 1 FROM scopes WHERE scope_id=jobs.scope_id AND revoked=0)", params![job_id,owner,now as i64,now.saturating_add(30000) as i64])?;
        if changed != 1 {
            return Err(StoreError::Conflict("job is not claimable".into()));
        }
        crate::job_authority_gate::validate_job(&tx, job_id, &[Permission::IndexWrite], strict)?;
        tx.commit()?;
        self.job(job_id)
    }

    /// 复核持久身份与实时权限并续租。参数：任务、owner、fence；返回：原代次续租或错误。
    pub(crate) fn heartbeat_job_authorized(
        &mut self,
        job_id: &str,
        owner: &str,
        fence: u64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::job_authority_gate::validate_job(&tx, job_id, &[Permission::IndexWrite], false)?;
        let now = Self::now_ms();
        let changed = tx.execute("UPDATE jobs SET heartbeat_unix_ms=?4,lease_expires_unix_ms=?5 WHERE job_id=?1 AND owner=?2 AND fencing_token=?3 AND state='running' AND cancel_requested=0 AND lease_expires_unix_ms>?4", params![job_id,owner,fence as i64,now as i64,now.saturating_add(30000) as i64])?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        crate::job_authority_gate::validate_job(&tx, job_id, &[Permission::IndexWrite], false)?;
        tx.commit()?;
        Ok(())
    }
}
