//! 扫描执行前只对账已经提交的事实；来源：原生 Rust RT-01。
use crate::{Engine, EngineError};
use diskgraph_store::{JobKind, JobRecord, JobState};

impl Engine {
    /// 参数：实际任务与拟认领 owner；返回：原提交事实的控制终态或尚未发布。
    /// 不采样、不续期原认证、不抢占活租约；回执损坏或身份冲突必须失败。
    pub(super) fn recover_scan_publication(
        &self,
        job_id: &str,
        owner: &str,
    ) -> Result<Option<JobRecord>, EngineError> {
        let job = self.control()?.job(job_id)?;
        if !matches!(job.kind, JobKind::Index | JobKind::Sync)
            || !matches!(job.state, JobState::Queued | JobState::Running)
        {
            return Ok(None);
        }
        // 对账只读取已提交事实：不能在认领前等待扫描准备/发布所持的共享写锁。
        // WAL 独立 reader 沿既有有限期限读取，活事务未提交的回执不会被当作事实。
        let receipt = self.revision_reader()?.scan_publication_receipt(job_id)?;
        match receipt {
            Some(receipt) => Ok(Some(
                self.control()?
                    .recover_committed_scan_job(job_id, owner, &receipt)?,
            )),
            None => Ok(None),
        }
    }
}
