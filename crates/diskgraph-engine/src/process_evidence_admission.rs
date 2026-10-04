//! Process 类型接入既有 runner 的安全边界；来源：Rust D42，原生 executor 接通前不借用 Scan。
use crate::{Engine, EngineError};
use diskgraph_core::{
    BusinessError, ProcessEvidenceFailure, ProcessEvidenceFailureCode as Code,
    ProcessEvidenceFailurePhase,
};
use diskgraph_store::{JobKind, JobRecord, JobState, StoreError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

impl Engine {
    /// 参数：实际认领的任务代次、取消标志和原始时钟；返回：授权/归属检查后的明确能力失败。
    /// 不触碰目标源，不创建 snapshot，不把未连接的产品执行器伪装成成功或范围重扫。
    pub(super) fn admit_process_execution(
        &self,
        job_id: &str,
        owner: &str,
        fence: u64,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<(), EngineError> {
        let mut control = self.control()?;
        control.with_job_fence(job_id, owner, fence, || Ok(()))?;
        let job = control.job(job_id)?;
        let input = control.process_evidence_job_input(job_id)?;
        if job.kind != JobKind::ProcessEvidence
            || &job.scope_id != input.scope_id()
            || &control.existing_server_id()? != input.server_id()
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(BusinessError::Conflict.into());
        }
        if started.elapsed() >= Duration::from_millis(input.limits().max_duration_ms()) {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Err(BusinessError::Unsupported.into())
    }
    /// 参数：任务及拟认领 owner；返回：同一唯一回执的历史完成事实，不延长执行权限或重采样。
    pub(super) fn recover_process_publication(
        &self,
        job_id: &str,
        owner: &str,
    ) -> Result<Option<JobRecord>, EngineError> {
        if self.control()?.job(job_id)?.kind != JobKind::ProcessEvidence {
            return Ok(None);
        }
        let receipt = self.graph()?.process_job_publication_receipt(job_id)?;
        match receipt {
            Some(receipt) => Ok(Some(
                self.control()?
                    .recover_committed_process_job(job_id, owner, &receipt)?,
            )),
            None => Ok(None),
        }
    }
}

/// 参数：真实入场失败与持久状态；返回：固定诊断，不猜测原始错误文本或暴露源路径。
pub(super) fn failure(error: &EngineError, state: JobState) -> ProcessEvidenceFailure {
    let code = if state == JobState::Cancelled {
        Code::Cancelled
    } else {
        match error {
            EngineError::Business(BusinessError::Unsupported) => Code::Unsupported,
            EngineError::Business(
                BusinessError::BudgetExceeded | BusinessError::ResourceExhausted,
            )
            | EngineError::Store(StoreError::BudgetExceeded) => Code::BudgetExceeded,
            EngineError::Business(BusinessError::PermissionDenied) => Code::PermissionDenied,
            EngineError::Business(BusinessError::Timeout) => Code::Timeout,
            EngineError::Business(BusinessError::Conflict)
            | EngineError::Store(StoreError::Conflict(_) | StoreError::StaleOwner) => {
                Code::Conflict
            }
            _ => Code::InternalError,
        }
    };
    ProcessEvidenceFailure::new(ProcessEvidenceFailurePhase::Admission, code)
}
