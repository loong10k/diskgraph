//! 持久扫描任务状态的有界授权读取；兼容 wire 由适配器保持。
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Decision, Permission, PrincipalId};
use diskgraph_store::{AuthorizationWithdrawalStatus, JobRecord};
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
        let decision = authorizer.decide(principal, &Permission::OperationView, &job.scope_id);
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if matches!(decision, Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        let control = self.control_until(deadline)?;
        let current = control.with_read_deadline(deadline, |control| {
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
        })?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        Ok(current)
    }
}
