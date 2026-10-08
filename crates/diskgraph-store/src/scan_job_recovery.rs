//! 条件结算已提交扫描，不重新授予工作权限；来源：原生 Rust RT-01。
use crate::{ControlStore, JobRecord, Result, ScanPublicationReceipt, StoreError};
use rusqlite::{TransactionBehavior, params};

impl ControlStore {
    /// 参数：实际任务、恢复 owner、从图库不可变读取的真实回执；返回：条件结算结果或身份/代次冲突。
    /// 可信内部入口，远程不得传入构造回执；活租约不抢占，原请求到期不否认已经提交的事实。
    pub fn recover_committed_scan_job(
        &mut self,
        job_id: &str,
        owner: &str,
        receipt: &ScanPublicationReceipt,
    ) -> Result<JobRecord> {
        receipt.validate()?;
        if owner.is_empty()
            || owner.len() > 256
            || job_id != receipt.job_id
            || self.existing_server_id()? != receipt.server_id
        {
            return Err(StoreError::InvalidGraph(
                "scan receipt server or job mismatch".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (kind, scope, principal, fence, state): (String, String, String, i64, String) = tx
            .query_row(
                "SELECT kind,scope_id,principal,fencing_token,state FROM jobs WHERE job_id=?1",
                [job_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
        if kind != receipt.kind.as_str()
            || scope != receipt.scope_id.as_str()
            || principal != receipt.principal.as_str()
            || fence < 0
            || (fence as u64) < receipt.publishing_fence
            || crate::job_authority_gate::read(&tx, job_id)? != receipt.authority
        {
            return Err(StoreError::InvalidGraph(
                "scan receipt provenance mismatch".into(),
            ));
        }
        if state == "completed" {
            tx.commit()?;
            return self.job(job_id);
        }
        let now = Self::now_ms();
        let changed=tx.execute("UPDATE jobs SET state='completed',owner=?2,heartbeat_unix_ms=?3,lease_expires_unix_ms=?3,fencing_token=fencing_token+1 WHERE job_id=?1 AND fencing_token=?4 AND fencing_token<9223372036854775807 AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?3))",params![job_id,owner,now as i64,fence])?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        tx.commit()?;
        self.job(job_id)
    }
}
