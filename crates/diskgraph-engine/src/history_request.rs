use crate::authority_expiry::check_authority_expiry;
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, Permission, PrincipalId, QueryBudget, QueryReadBudget,
};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Instant;

impl Engine {
    /// 双侧历史读取共用原始期限，编码之后检查实际双侧 scope/grant。
    /// 参数：left/right 为固定 revision，身份与 deadline 为同请求，consumer/finish 不得读取其他快照。
    /// 返回：预算内结果；任何错误或期限 partial 都必须先通过真实末段授权。
    #[allow(clippy::too_many_arguments)] // 双侧上下文和共享消费者/编码阶段均为独立必需输入。
    pub(super) fn with_history_readers_until<T>(
        &self,
        left: &str,
        right: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        budget: QueryBudget,
        consumer: impl FnOnce(
            &SqliteSnapshotStore,
            &str,
            &SqliteSnapshotStore,
            &str,
            &mut QueryReadBudget,
            bool,
        ) -> Result<T, EngineError>,
        mut finish: impl FnMut(&mut T, bool) -> Result<(), EngineError>,
    ) -> Result<T, EngineError> {
        let expiry = authorizer.expires_at_unix_seconds();
        check_authority_expiry(expiry)?;
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        let left_reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let right_reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        // 在任何能力回调前读实际归属并绑定双侧撤权，复用原账本而不重复查图。
        let left_owner = left_reader
            .revision_ownership_with_budget(left, &mut reads)?
            .ok_or(BusinessError::PermissionDenied)?;
        let right_owner = right_reader
            .revision_ownership_with_budget(right, &mut reads)?
            .ok_or(BusinessError::PermissionDenied)?;
        let left_scope = diskgraph_core::ScopeId::new(left_owner.1.clone())
            .map_err(|_| BusinessError::PermissionDenied)?;
        let right_scope = diskgraph_core::ScopeId::new(right_owner.1.clone())
            .map_err(|_| BusinessError::PermissionDenied)?;
        let control = self.control_until(deadline)?;
        let withdrawals = control
            .with_read_deadline(deadline, |control| {
                crate::request_metadata_withdrawals::RequestMetadataWithdrawals::capture(
                    control,
                    principal,
                    &[&left_scope, &right_scope],
                )
            })
            .map_err(crate::relation_request::terminal_control_error)?;
        drop(control);
        self.authorize_revision_owner_until(
            left,
            Some(left_owner),
            None,
            principal,
            authorizer,
            deadline,
        )?;
        self.authorize_revision_owner_until(
            right,
            Some(right_owner),
            None,
            principal,
            authorizer,
            deadline,
        )?;
        // 双侧都已按本服务器真实归属授权；同 ScopeId 绑定同一不可变注册根。
        // 目标准入错误与 consumer 错误一起经过原双侧末检，不提前返回错误或部分结果。
        let result = (|| {
            check_authority_expiry(expiry)?;
            let left_snapshot = left_reader.revision_snapshot_with_budget(left, &mut reads)?;
            let right_snapshot = right_reader.revision_snapshot_with_budget(right, &mut reads)?;
            check_authority_expiry(expiry)?;
            consumer(
                &left_reader,
                &left_snapshot,
                &right_reader,
                &right_snapshot,
                &mut reads,
                left_scope == right_scope,
            )
        })();
        // 仅测试的读后同步点不阻塞授权回调，也不改变生产期限或请求状态。
        #[cfg(test)]
        crate::relation_request_tests::after_read(deadline);
        let scopes = [&left_scope, &right_scope];
        // 两侧每轮共用固定授权窗口；只读过滤归属，不续期历史迭代与编码预算。
        let ownerships = || {
            check_authority_expiry(expiry)?;
            let control =
                crate::authorization_phase_diagnostic::observe("history_terminal_lock", || {
                    self.try_control_store()?
                        .ok_or_else(|| BusinessError::BudgetExceeded.into())
                })?;
            check_authority_expiry(expiry)?;
            // 能力回调可能消耗原查询期限；每轮归属观察才开始独立的有限窗口。
            // 同轮双侧共享此窗口，原 reads/deadline 不刷新，迟到结果仍由 finish 拒绝。
            let authorization_deadline = Instant::now()
                .checked_add(crate::terminal_authorization_windows::DATABASE_WINDOW)
                .ok_or(BusinessError::InvalidArgument)?;
            #[cfg(test)]
            crate::history_relation_withdrawal_tests::before_terminal_sql(authorization_deadline);
            let control_authorization =
                crate::authorization_phase_diagnostic::observe("history_terminal_control", || {
                    control
                        .with_read_deadline(authorization_deadline, |control| {
                            for scope in &scopes {
                                if control.scope_revoked(scope)?
                                    || control.live_permission(
                                        principal,
                                        &Permission::MetadataRead,
                                        scope,
                                    )? == Some(false)
                                {
                                    return Err(EngineError::Business(
                                        BusinessError::PermissionDenied,
                                    ));
                                }
                            }
                            withdrawals.check_after_live_authorization(control)
                        })
                        .map_err(crate::relation_request::terminal_control_error)
                });
            self.prioritize_revision_quarantine(
                control_authorization,
                &[(left, &left_scope), (right, &right_scope)],
                &control,
                authorization_deadline,
            )?;
            self.require_terminal_revision_ownerships(
                &[(left, &left_scope), (right, &right_scope)],
                &control,
                authorization_deadline,
            )?;
            check_authority_expiry(expiry)
        };
        let authorize = || {
            let observed = (|| {
                let timely =
                    self.require_terminal_relations(authorizer, principal, &scopes, expiry)?;
                ownerships()?;
                if !timely {
                    return Err(EngineError::Business(BusinessError::BudgetExceeded));
                }
                Ok(())
            })();
            withdrawals.prioritize_terminal_budget(self, observed)
        };
        authorize()?;
        let mut result = result?;
        let expired = Instant::now() >= deadline;
        let encoded = finish(&mut result, expired);
        // 编码不持控制锁；能力回调全部结束后再次读取每一侧实时授权与归属。
        authorize()?;
        encoded?;
        if !expired && Instant::now() >= deadline {
            let encoded = finish(&mut result, true);
            authorize()?;
            encoded?;
        }
        check_authority_expiry(expiry)?;
        Ok(result)
    }
}
