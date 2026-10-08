//! 内容初始授权和实际范围解析共用原始期限；来源：DiskGraph CT-01/02。
use super::InspectionRequest;
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Decision, Permission};
use diskgraph_store::ScopeRecord;
use std::time::Instant;

impl Engine {
    /// 参数：request 为原内容主体/范围，authorizer 为锁外能力，deadline 为原期限。
    /// 返回：当前获准范围或原授权/预算/存储错误，不在控制锁内调用能力来源。
    pub(super) fn require_content_initial_until(
        &self,
        request: &InspectionRequest<'_>,
        authorizer: &dyn Authorizer,
        deadline: Instant,
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
