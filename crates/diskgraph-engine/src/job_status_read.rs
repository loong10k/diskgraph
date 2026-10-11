//! 持久扫描任务状态的有界授权读取；兼容 wire 由适配器保持。
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Decision, Permission, PrincipalId};
use diskgraph_store::{AuthorizationWithdrawalStatus, ControlStore, JobRecord};
use std::time::Instant;

impl Engine {
    /// 按实际任务 scope 与实时操作查看权限读取状态，回调保持在控制锁外。
    /// 参数：job_id 为持久任务，principal/authorizer 为请求能力，deadline 为原期限。
    /// 返回：获准的当前记录；撤权、scope 改变、到期或查询失败拒绝结果。
    pub fn job_status_authorized_until(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<JobRecord, EngineError> {
        self.with_job_status_authorized_until(job_id, principal, authorizer, deadline, |job| {
            Ok(job.clone())
        })
    }

    /// 在原任务授权见证内编码响应并复验实时OperationView权限。
    /// 参数：job_id为实际任务，principal/authorizer为原主体和能力，deadline为原期限，consumer只准备未公开响应。
    /// 返回：编码结果或撤权、归属冲突、到期及预算失败；消费者不持控制库锁。
    pub fn with_job_status_authorized_until<T>(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        consumer: impl FnOnce(&JobRecord) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let control = self.control_until(deadline)?;
        let (job, watch, generation) = control.with_read_deadline(deadline, |control| {
            let job = control.job(job_id)?;
            if control.scope_revoked(&job.scope_id)? {
                return Err(BusinessError::PermissionDenied.into());
            }
            let watch = control.watch_authorization_withdrawal(
                principal,
                &job.scope_id,
                &Permission::OperationView,
            )?;
            Ok::<_, EngineError>((job, watch, control.authorization_generation()?))
        })?;
        drop(control);
        let capability_deadline =
            Instant::now() + crate::terminal_authorization_windows::CAPABILITY_WINDOW;
        let decision = authorizer.decide(principal, &Permission::OperationView, &job.scope_id);
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if matches!(decision, Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        // 同一个原始撤权见证用于读前与编码后观察，禁止通过重新捕获掩盖撤权再恢复。
        let observe = |control: &ControlStore, decision: Decision| {
            match watch.as_ref().map(|watch| watch.status_for(control)) {
                Some(AuthorizationWithdrawalStatus::Withdrawn) => {
                    return Err(BusinessError::PermissionDenied.into());
                }
                Some(AuthorizationWithdrawalStatus::Invalidated) => {
                    return Err(BusinessError::Conflict.into());
                }
                _ => {}
            }
            Self::require_decision_with_control(
                control,
                decision,
                principal,
                &Permission::OperationView,
                &job.scope_id,
            )?;
            if control.scope_revoked(&job.scope_id)? {
                return Err(BusinessError::PermissionDenied.into());
            }
            // 未知通知能力下，任何授权代次变化均保守拒绝，不声称知道具体撤权原因。
            if watch.is_none() && control.authorization_generation()? != generation {
                return Err(BusinessError::Conflict.into());
            }
            let current = control.job(job_id)?;
            if current.scope_id != job.scope_id {
                return Err(BusinessError::Conflict.into());
            }
            Ok::<_, EngineError>(current)
        };
        // 迟到能力只能参与有界的拒绝原因核验，不能获得新的业务读取期限。
        let observation_deadline = if Instant::now() >= capability_deadline {
            deadline.min(Instant::now() + crate::terminal_authorization_windows::DATABASE_WINDOW)
        } else {
            deadline
        };
        let control = self.control_until(observation_deadline)?;
        let current = control
            .with_read_deadline(observation_deadline, |control| observe(control, decision))?;
        drop(control);
        // 先保留本请求已提交撤权/代次变化的拒绝原因，再处理迟到能力。
        // 迟到的 Allow 仍不能进入消费者，50ms 与原请求期限均不续期。
        if Instant::now() >= capability_deadline || Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let result = consumer(&current);
        crate::authority_expiry::check_authority_expiry(expiry)?;
        let capability_deadline =
            Instant::now() + crate::terminal_authorization_windows::CAPABILITY_WINDOW;
        let decision = authorizer.decide(principal, &Permission::OperationView, &job.scope_id);
        let timely = Instant::now() < capability_deadline;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if matches!(decision, Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        let terminal = Instant::now() + crate::terminal_authorization_windows::DATABASE_WINDOW;
        let control = match self.try_control_store()? {
            Some(control) => control,
            None => self.control_until(terminal.min(deadline))?,
        };
        control.with_read_deadline(terminal, |control| observe(control, decision))?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if !timely || Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        result
    }
}
