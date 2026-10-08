//! 内容初始授权和实际范围解析共用原始期限；来源：DiskGraph CT-01/02。
use super::InspectionRequest;
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Decision, Permission};
use diskgraph_store::ScopeRecord;
use std::time::Instant;

impl Engine {
    /// 沿原期限复核内容授权，并在同一控制窗口检查全过程撤权。
    /// 参数：withdrawal 为首次回调前捕获的见证；返回：范围或拒权、冲突、预算错误。
    pub(super) fn require_content_withdrawal_until(
        &self,
        request: &InspectionRequest<'_>,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        withdrawal: &super::content_withdrawal::ContentWithdrawal,
    ) -> Result<ScopeRecord, EngineError> {
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let decision = authorizer.decide(
            request.principal,
            &Permission::ContentRead,
            request.scope_id,
        );
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if matches!(decision, Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let control = self.control_until(deadline)?;
        let record = control
            .with_read_deadline(deadline, |control| {
                Self::require_decision_with_control(
                    control,
                    decision,
                    request.principal,
                    &Permission::ContentRead,
                    request.scope_id,
                )?;
                let record = control.scope(request.scope_id)?;
                if record.revoked {
                    return Err(BusinessError::PermissionDenied.into());
                }
                withdrawal.check_after_live_authorization(control)?;
                Ok::<_, EngineError>(record)
            })
            .map_err(|error| match error {
                EngineError::Store(error)
                    if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                        || error.is_busy()
                        || error.is_interrupted() =>
                {
                    BusinessError::BudgetExceeded.into()
                }
                other => other,
            })?;
        crate::authority_expiry::check_authority_expiry(expiry)?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(record)
    }
}
