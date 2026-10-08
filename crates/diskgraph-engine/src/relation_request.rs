//! 关系请求复用真实 revision 授权与独立 reader，返回前在同一 control guard 复检。
use crate::authority_expiry::check_authority_expiry;
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, Permission, PrincipalId, QueryBudget, QueryReadBudget, ScopeId,
    TruncationReason,
};
use diskgraph_store::{RevisionEvidenceReader, SqliteSnapshotStore};
use std::time::Instant;

impl Engine {
    /// 执行一次真实归属查询，有限完成结果后复核授权与期限。
    /// 参数：revision/身份、原期限/账本、范围断言及消费/编码只用于该请求。
    /// 返回：已复核结果或失败；真实格式错误不改写，预算失败仍终检授权。
    #[allow(clippy::too_many_arguments)] // 实际身份、范围、期限、额度与两阶段消费者均独立必需。
    pub(super) fn with_relation_reader_until<T>(
        &self,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        budget: QueryBudget,
        expected_scope: Option<&ScopeId>,
        consumer: impl FnOnce(
            &SqliteSnapshotStore,
            Option<&RevisionEvidenceReader<'_>>,
            &mut QueryReadBudget,
        ) -> Result<T, EngineError>,
        mut finish: impl FnMut(&mut T, bool) -> Result<(), EngineError>,
    ) -> Result<T, EngineError> {
        let expiry = authorizer.expires_at_unix_seconds();
        check_authority_expiry(expiry)?;
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        #[cfg(test)]
        crate::relation_query_diagnostics_tests::mark("reader_open");
        let owner = reader
            .revision_ownership_with_budget(revision, &mut reads)?
            .ok_or(BusinessError::PermissionDenied)?;
        let scope = ScopeId::new(owner.1.clone()).map_err(|_| BusinessError::PermissionDenied)?;
        let control = self.control_until(deadline)?;
        let withdrawals = control
            .with_read_deadline(deadline, |control| {
                crate::request_metadata_withdrawals::RequestMetadataWithdrawals::capture(
                    control,
                    principal,
                    &[&scope],
                )
            })
            .map_err(terminal_control_error)?;
        drop(control);
        self.authorize_revision_owner_until(
            Some(owner),
            expected_scope,
            principal,
            authorizer,
            deadline,
        )?;
        #[cfg(test)]
        crate::relation_query_diagnostics_tests::mark("initial_authorization");
        // 必需目标与消费者共用最初余额；准备失败同样进入真实范围的末段授权。
        let result = (|| {
            check_authority_expiry(expiry)?;
            let evidence = match reader.revision_evidence_with_budget(revision, &mut reads) {
                Ok(evidence) => Some(evidence),
                Err(diskgraph_store::StoreError::BudgetExceeded)
                    if reads.stopped() == Some(TruncationReason::Deadline) =>
                {
                    None
                }
                Err(error) if error.is_interrupted() && Instant::now() >= deadline => {
                    reads.stop(TruncationReason::Deadline);
                    None
                }
                Err(error) => return Err(error.into()),
            };
            check_authority_expiry(expiry)?;
            consumer(&reader, evidence.as_ref(), &mut reads)
        })();
        #[cfg(test)]
        crate::relation_query_diagnostics_tests::mark("data_read");
        // 仅测试的读后同步点不持 control guard，不添加生产回调或共享请求状态。
        #[cfg(test)]
        crate::relation_request_tests::after_read(deadline);
        // 每轮能力回调后开始固定归属窗口；不续期结果读取或编码期限。
        let ownership = || {
            check_authority_expiry(expiry)?;
            let control = self
                .try_control_store()?
                .ok_or(BusinessError::BudgetExceeded)?;
            check_authority_expiry(expiry)?;
            let authorization_deadline = Instant::now()
                .checked_add(std::time::Duration::from_millis(50))
                .ok_or(BusinessError::InvalidArgument)?;
            control
                .with_read_deadline(authorization_deadline, |control| {
                    if control.scope_revoked(&scope)?
                        || control.live_permission(principal, &Permission::MetadataRead, &scope)?
                            == Some(false)
                    {
                        return Err(EngineError::Business(BusinessError::PermissionDenied));
                    }
                    withdrawals.check_after_live_authorization(control)
                })
                .map_err(terminal_control_error)?;
            self.require_terminal_revision_ownership(
                revision,
                &scope,
                &control,
                authorization_deadline,
            )?;
            check_authority_expiry(expiry)
        };
        let authorize = || {
            let timely = self.observe_terminal_relation(authorizer, principal, &scope, expiry)?;
            ownership()?;
            if !timely {
                return Err(EngineError::Business(BusinessError::BudgetExceeded));
            }
            Ok(())
        };
        authorize()?;
        #[cfg(test)]
        crate::relation_query_diagnostics_tests::mark("terminal_before_encode");
        let mut result = result?;
        let expired = Instant::now() >= deadline;
        let encoded = finish(&mut result, expired);
        #[cfg(test)]
        crate::relation_query_diagnostics_tests::mark("encode");
        // 编码不持控制锁；编码后重新观察实时授权与实际归属。
        authorize()?;
        #[cfg(test)]
        crate::relation_query_diagnostics_tests::mark("terminal_after_encode");
        encoded?;
        if !expired && Instant::now() >= deadline {
            let encoded = finish(&mut result, true);
            authorize()?;
            encoded?;
        }
        check_authority_expiry(expiry)?;
        Ok(result)
    }

    /// 分阶段观察关系/历史末段权限，不得在既有 SQL deadline guard 中调用。
    /// 参数：authorizer/principal/scope 为真实身份，expiry 是请求开始的固定值；各 SQL 阶段非阻塞取锁。
    /// 返回：授权允许是否及时；调用方须先复核其他侧和归属，再将迟到允许拒为预算失败。
    pub(super) fn observe_terminal_relation(
        &self,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scope: &ScopeId,
        expiry: Option<u64>,
    ) -> Result<bool, EngineError> {
        check_authority_expiry(expiry)?;
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        check_authority_expiry(expiry)?;
        let before_callback = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        control
            .with_read_deadline(before_callback, |control| {
                if control.scope_revoked(scope)? {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                Ok(())
            })
            .map_err(terminal_control_error)?;
        drop(control);
        check_authority_expiry(expiry)?;
        // 宿主回调不持控制锁或 SQL handler；回调后重新观察实时权限。
        let capability_deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let decision = authorizer.decide(principal, &Permission::MetadataRead, scope);
        let capability_timely = Instant::now() < capability_deadline;
        #[cfg(test)]
        crate::relation_request_tests::after_terminal_callback();
        check_authority_expiry(expiry)?;
        if matches!(decision, diskgraph_core::Decision::Denied(_)) {
            return Err(BusinessError::PermissionDenied.into());
        }
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        check_authority_expiry(expiry)?;
        let after_callback = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        control
            .with_read_deadline(after_callback, |control| {
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
            .map_err(terminal_control_error)?;
        check_authority_expiry(expiry)?;
        // 不在此提前返回预算错误；调用方还须观察其他侧撤权及新鲜 revision 隔离。
        Ok(capability_timely)
    }

    /// 完成全部能力回调后，再纯读取每一侧的持久授权。
    /// 参数：authorizer/principal/scopes 为真实身份与范围，expiry 沿用原值；回调不持控制锁。
    /// 返回：各侧均允许时的及时性；真实拒权优先，迟到允许由调用方完成归属观察后拒绝。
    pub(super) fn require_terminal_relations(
        &self,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scopes: &[&ScopeId],
        expiry: Option<u64>,
    ) -> Result<bool, EngineError> {
        check_authority_expiry(expiry)?;
        let mut timely = true;
        for scope in scopes {
            timely &= self.observe_terminal_relation(authorizer, principal, scope, expiry)?;
        }
        // 不再次调用能力授权器，避免最后一个回调继续使前侧复检失效。
        // 全部回调结束后取得新 guard；独立 SQLite 连接仍不受此 mutex 冻结。
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        check_authority_expiry(expiry)?;
        let observation_deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        control
            .with_read_deadline(observation_deadline, |control| {
                for scope in scopes {
                    if control.scope_revoked(scope)?
                        || control.live_permission(principal, &Permission::MetadataRead, scope)?
                            == Some(false)
                    {
                        return Err(EngineError::Business(BusinessError::PermissionDenied));
                    }
                }
                Ok(())
            })
            .map_err(terminal_control_error)?;
        check_authority_expiry(expiry)?;
        Ok(timely)
    }

    /// 适配器完成真实 envelope 编码后再复核 revision 权限与共同期限。
    /// 参数：revision/principal/authorizer 为真实资源请求，deadline 为最外层开始的期限。
    /// 返回：Ok(true) 仍有时间，Ok(false) 已到期；先检查实时撤权，拒绝完整或部分数据。
    pub fn finalize_revision_read_until(
        &self,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<bool, EngineError> {
        self.finalize_revisions_read_until(&[revision], principal, authorizer, deadline)
    }

    /// 对成组 revision 完成全部能力回调后，再复核所有持久 scope/grant。
    /// 参数：revisions 为真实读取列表，principal/authorizer 与 deadline 沿用整个请求。
    /// 返回：Ok(true) 尚未过期、Ok(false) 已过期；任一侧撤权均拒绝全部数据。
    pub fn finalize_revisions_read_until(
        &self,
        revisions: &[&str],
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<bool, EngineError> {
        if revisions.is_empty() {
            return Err(BusinessError::InvalidArgument.into());
        }
        let expiry = authorizer.expires_at_unix_seconds();
        check_authority_expiry(expiry)?;
        // 实时授权的前后 SQL 与能力回调各有固定窗口，不续期数据查询，也不向消费者交出此连接。
        // 数据已到期时仍须拒绝撤权后的前缀；观察超时则拒绝全部结果。
        let observation_deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, observation_deadline, None)?;
        let ownerships = revisions
            .iter()
            .map(|revision| {
                let (server, scope) = reader
                    .revision_ownership(revision)?
                    .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
                let scope = ScopeId::new(scope)
                    .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
                Ok((server, scope))
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let control = self
            .try_control_store()?
            .ok_or(EngineError::Business(BusinessError::BudgetExceeded))?;
        check_authority_expiry(expiry)?;
        control.with_read_deadline(observation_deadline, |control| {
            let server_id = control.existing_server_id()?;
            for (server, scope) in &ownerships {
                if server != server_id.as_str() || control.scope_revoked(scope)? {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
            }
            Ok::<(), EngineError>(())
        })?;
        drop(reader);
        drop(control);
        check_authority_expiry(expiry)?;
        // 宿主回调不消耗后续新鲜 SQL 的窗口；迟到允许仍不能提交部分数据。
        let capability_deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let decisions = ownerships
            .iter()
            .map(|(_, scope)| {
                check_authority_expiry(expiry)?;
                let decision = authorizer.decide(principal, &Permission::MetadataRead, scope);
                check_authority_expiry(expiry)?;
                Ok::<_, EngineError>(decision)
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let capability_timely = Instant::now() < capability_deadline;
        if decisions
            .iter()
            .any(|decision| matches!(decision, diskgraph_core::Decision::Denied(_)))
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        check_authority_expiry(expiry)?;
        let after_callback = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        control.with_read_deadline(after_callback, |control| {
            let current_server = control.existing_server_id()?;
            for ((server, scope), decision) in ownerships.iter().zip(decisions) {
                if server != current_server.as_str() {
                    return Err(BusinessError::PermissionDenied.into());
                }
                Self::require_decision_with_control(
                    control,
                    decision,
                    principal,
                    &Permission::MetadataRead,
                    scope,
                )?;
            }
            // 所有能力回调结束后纯读每侧授权，后侧回调不能撤销已检查的前侧后逃逸。
            for (_, scope) in &ownerships {
                if control.scope_revoked(scope)?
                    || control.live_permission(principal, &Permission::MetadataRead, scope)?
                        == Some(false)
                {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
            }
            Ok::<(), EngineError>(())
        })?;
        check_authority_expiry(expiry)?;
        let reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, after_callback, None)?;
        for (revision, (server, scope)) in revisions.iter().zip(&ownerships) {
            if !reader.revision_ownership_matches(revision, server, scope.as_str())? {
                return Err(BusinessError::PermissionDenied.into());
            }
        }
        check_authority_expiry(expiry)?;
        if Instant::now() >= after_callback || !capability_timely {
            return Err(BusinessError::BudgetExceeded.into());
        }
        check_authority_expiry(expiry)?;
        Ok(Instant::now() < deadline)
    }
}

/// 将控制 SQL 的期限、中断及锁等待映射为预算拒绝，其他原错误不改写。
/// 映射终检数据库执行预算、锁等待与中断错误。
/// 参数：error 为原引擎错误；返回：预算类转为 BudgetExceeded，其余错误保持不变。
pub(super) fn terminal_control_error(error: EngineError) -> EngineError {
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
