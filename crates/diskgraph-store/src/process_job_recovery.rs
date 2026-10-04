//! 结算图库已提交事实；来源：原生 Rust RT-01，不授予新的采集权限。
use crate::{ControlStore, JobRecord, Result, StoreError};
use diskgraph_core::ProcessJobPublicationReceipt;
use rusqlite::{TransactionBehavior, params};

impl ControlStore {
    /// 参数：实际任务、恢复 owner、Engine 从图库真实不可变查询取得的回执；返回：条件结算或冲突。
    /// 这是可信内部协调入口，不能接受客户端构造的回执。原到期/撤权不否认已提交事实。
    /// 活租约不抢占；两库各自提交，不声称跨库原子性。
    pub fn recover_committed_process_job(
        &mut self,
        job_id: &str,
        owner: &str,
        receipt: &ProcessJobPublicationReceipt,
    ) -> Result<JobRecord> {
        if owner.is_empty() || owner.len() > 256 || job_id != receipt.job_id() {
            return Err(StoreError::Conflict(
                "invalid Process receipt recovery owner".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let input = crate::process_job_input_codec::read(&tx, job_id)?;
        if input != *receipt.input() || input.digest() != receipt.input_sha256() {
            return Err(StoreError::InvalidGraph(
                "Process receipt target mismatch".into(),
            ));
        }
        // 仅重验原身份的不可变结构和真实主体；绝不续期或借新 token 创建工作。
        if crate::job_authority_gate::read(&tx, job_id)?.is_none() {
            return Err(StoreError::InvalidGraph(
                "Process receipt request provenance missing".into(),
            ));
        }
        let (process, completed, fence): (bool, bool, i64) = tx.query_row(
            "SELECT kind='process_evidence',state='completed',fencing_token FROM jobs WHERE job_id=?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if !process || fence < 0 || (fence as u64) < receipt.publishing_fence() {
            return Err(StoreError::InvalidGraph(
                "Process receipt publishing fence mismatch".into(),
            ));
        }
        if completed {
            tx.commit()?;
            return self.job(job_id);
        }
        let now = Self::now_ms();
        let changed=tx.execute("UPDATE jobs SET state='completed',owner=?2,heartbeat_unix_ms=?3,
            lease_expires_unix_ms=?3,fencing_token=fencing_token+1 WHERE job_id=?1 AND fencing_token=?4
            AND fencing_token<9223372036854775807 AND (state='queued' OR (state='running' AND lease_expires_unix_ms<=?3))",
            params![job_id,owner,now as i64,fence])?;
        if changed != 1 {
            return Err(StoreError::StaleOwner);
        }
        tx.commit()?;
        self.job(job_id)
    }
}
