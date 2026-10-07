//! 初始授权的控制锁、SQL 与能力回调期限边界。
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};

impl Engine {
    // 初始授权只使用原请求期限；控制锁和控制 SQL 不另建宽限窗口。
    /// 沿原请求期限检查持久归属、实时权限与能力交集。
    /// 参数：ownership 是图查询的真实归属，expected_scope 为一致性断言，身份与期限不可刷新。
    /// 返回：实际 scope；已观察拒权优先，迟到允许及控制预算耗尽拒绝。
    pub(super) fn authorize_revision_owner_until(
        &self,
        ownership: Option<(String, String)>,
        expected_scope: Option<&ScopeId>,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: std::time::Instant,
    ) -> Result<ScopeId, EngineError> {
        let (server, scope) = ownership.ok_or(BusinessError::PermissionDenied)?;
        let scope = ScopeId::new(scope).map_err(|_| BusinessError::PermissionDenied)?;
        let control = self.control_until(deadline)?;
        let authorization = control.with_read_deadline(deadline, |control| {
            if server != control.existing_server_id()?.as_str()
                || expected_scope.is_some_and(|expected| expected != &scope)
            {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            if control.scope_revoked(&scope)? {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            Ok(())
        });
        match authorization {
            Err(EngineError::Store(error))
                if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                    || error.is_interrupted()
                    || error.is_busy() =>
            {
                return Err(BusinessError::BudgetExceeded.into());
            }
            other => other?,
        }
        // 宿主回调不运行在 SQLite handler 内；明确拒权始终拒绝，迟到允许不续租 SQL。
        let decision = authorizer.decide(principal, &Permission::MetadataRead, &scope);
        if matches!(decision, diskgraph_core::Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let authorization = control.with_read_deadline(deadline, |control| {
            Self::require_decision_with_control(
                control,
                decision,
                principal,
                &Permission::MetadataRead,
                &scope,
            )?;
            if control.scope_revoked(&scope)? {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            Ok(())
        });
        match authorization {
            Err(EngineError::Store(error))
                if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                    || error.is_interrupted()
                    || error.is_busy() =>
            {
                return Err(BusinessError::BudgetExceeded.into());
            }
            other => other?,
        }
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(scope)
    }
}

impl Engine {
    /// 在可信 reader 的原期限内核对能力与实时持久授权。
    /// 参数：control 为既有控制锁，身份与 scope 为实际归属，deadline 不得刷新。
    /// 返回：允许、明确拒权或预算失败；回调不得继承 SQLite VM handler。
    pub(super) fn require_reader_capability_until(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scope: &ScopeId,
        deadline: std::time::Instant,
    ) -> Result<(), EngineError> {
        control
            .with_read_deadline(deadline, |control| {
                if control.scope_revoked(scope)? {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                Ok(())
            })
            .map_err(reader_control_error)?;
        // 宿主能力回调处于两个 SQL 阶段之间；迟到允许不能延长后续观察期限。
        let decision = authorizer.decide(principal, &Permission::MetadataRead, scope);
        if matches!(decision, diskgraph_core::Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        control
            .with_read_deadline(deadline, |control| {
                Self::require_decision_with_control(
                    control,
                    decision,
                    principal,
                    &Permission::MetadataRead,
                    scope,
                )?;
                if control.scope_revoked(scope)? {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                Ok(())
            })
            .map_err(reader_control_error)
    }
}

// 只映射查询执行期限与锁等待；其他损坏及权限错误保留原含义。
fn reader_control_error(error: EngineError) -> EngineError {
    match error {
        EngineError::Store(error)
            if matches!(error, diskgraph_store::StoreError::BudgetExceeded)
                || error.is_interrupted()
                || error.is_busy() =>
        {
            BusinessError::BudgetExceeded.into()
        }
        other => other,
    }
}
