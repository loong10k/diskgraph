use crate::job_execution_stop_reason::JobExecutionStopReason;
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
    stop_reason: Option<&'a JobExecutionStopReason>,
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
            stop_reason: None,
        }
    }

    /// 参数：reason 为同执行代次 keeper 的原错误槽；返回：沿原门禁时钟的借用检查器。
    /// 不新增 owner 或授权能力，旧独立门禁构造保持不变。
    pub(super) fn with_stop_reason(mut self, reason: &'a JobExecutionStopReason) -> Self {
        self.stop_reason = Some(reason);
        self
    }

    fn check_fast(&self) -> Result<(), EngineError> {
        if let Some(authority) = self.authority {
            authority.validate_at(crate::job_authorization::unix_seconds()?)?;
        }
        if self.cancel.load(Ordering::SeqCst) {
            return Err(self
                .stop_reason
                .and_then(JobExecutionStopReason::take)
                .unwrap_or_else(|| BusinessError::Conflict.into()));
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
        let deadline = self
            .started
            .checked_add(Duration::from_millis(
                self.engine.scan_budget.max_duration_ms,
            ))
            .ok_or(BusinessError::BudgetExceeded)?;
        let mut control = loop {
            // 锁竞争期间仍检查本代取消、原停止原因和认证到期，不能只等待扫描总期限。
            self.check_fast()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(BusinessError::BudgetExceeded.into());
            }
            if let Some(control) = self.engine.try_control_store()? {
                break control;
            }
            #[cfg(test)]
            crate::scan_observation_deadline_tests::waiting_for_control();
            std::thread::sleep(remaining.min(Duration::from_millis(1)));
        };
        self.check_fast()?;
        // 同一原期限约束锁等待与实时标量检查，不为撤权轮询重复解码整个 scope。
        control
            .with_read_deadline(deadline, |control| {
                if control.scope_revoked(&self.job.scope_id)? {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
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
                Ok(())
            })
            .map_err(observation_control_error)?;
        // 先退出只读 progress guard，再用既有有期限 fence 事务；不能嵌套连接 handler。
        control
            .with_job_fence_until(
                &self.job.job_id,
                &self.job.owner,
                self.job.fencing_token,
                deadline,
                || Ok(()),
            )
            .map_err(|error| observation_control_error(error.into()))?;
        drop(control);
        self.check_fast()?;
        self.checked.set(Some(Instant::now()));
        Ok(())
    }
}

// 仅预算、busy 和中断统一为扫描预算错误；失权、损坏及 stale owner 保留原分类。
fn observation_control_error(error: EngineError) -> EngineError {
    match error {
        EngineError::Store(error)
            if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                || error.is_busy()
                || error.is_interrupted() =>
        {
            BusinessError::BudgetExceeded.into()
        }
        other => other,
    }
}
