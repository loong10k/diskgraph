use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, JobRequestAuthority, Permission};
use diskgraph_store::JobRecord;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 独立原生采样沿用整次任务期限、取消和实时授权，不持有数据库锁。
/// 来源：DiskGraph 原生 Rust D31；无 Java 对应对象。
pub(super) struct ScanObservationGuard<'a> {
    engine: &'a Engine,
    job: &'a JobRecord,
    authority: Option<&'a JobRequestAuthority>,
    cancel: &'a AtomicBool,
    started: Instant,
    checked: Cell<Option<Instant>>,
}

impl<'a> ScanObservationGuard<'a> {
    /// 创建任务局部采样门禁。参数：engine/job/cancel 为当前代次，authority 为持久身份，started 为整次扫描起点。
    /// 返回：不执行 I/O、不新增授权 owner 的协作检查器。
    pub(super) fn new(
        engine: &'a Engine,
        job: &'a JobRecord,
        authority: Option<&'a JobRequestAuthority>,
        cancel: &'a AtomicBool,
        started: Instant,
    ) -> Self {
        Self {
            engine,
            job,
            authority,
            cancel,
            started,
            checked: Cell::new(None),
        }
    }

    fn check_fast(&self) -> Result<(), EngineError> {
        if let Some(authority) = self.authority {
            authority.validate_at(crate::job_authorization::unix_seconds()?)?;
        }
        if self.cancel.load(Ordering::SeqCst) {
            return Err(BusinessError::Conflict.into());
        }
        if self.started.elapsed() > Duration::from_millis(self.engine.scan_budget.max_duration_ms) {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(())
    }

    /// 每次原生调用前后检查。参数：无；返回：允许或取消、超时、撤权及 fence 失效。
    /// 快速门禁每次执行；控制库至多复用 20ms，避免每个路径组件重复开启事务。
    pub(super) fn check(&self) -> Result<(), EngineError> {
        self.check_fast()?;
        if self
            .checked
            .get()
            .is_none_or(|last| last.elapsed() >= Duration::from_millis(20))
        {
            self.check_now()?;
        }
        Ok(())
    }

    /// 批次和发布前立即复验持久状态。参数：无；返回：当前真实任务授权或拒绝。
    /// 原生 I/O 不在本方法持有的控制事务内执行；发布自身仍使用原子 fence 事务。
    pub(super) fn check_now(&self) -> Result<(), EngineError> {
        self.check_fast()?;
        let mut control = self.engine.control()?;
        self.check_fast()?;
        if control.scope(&self.job.scope_id)?.revoked {
            return Err(BusinessError::PermissionDenied.into());
        }
        if control.cancellation_requested(&self.job.job_id, self.job.fencing_token)? {
            return Err(BusinessError::Conflict.into());
        }
        if control.policy_state()?.is_some()
            && control.live_permission(
                &self.job.principal,
                &Permission::IndexWrite,
                &self.job.scope_id,
            )? != Some(true)
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        control.with_job_fence(
            &self.job.job_id,
            &self.job.owner,
            self.job.fencing_token,
            || Ok(()),
        )?;
        drop(control);
        self.check_fast()?;
        self.checked.set(Some(Instant::now()));
        Ok(())
    }
}
