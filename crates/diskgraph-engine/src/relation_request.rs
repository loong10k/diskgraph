//! 关系请求复用真实 revision 授权与独立 reader，返回前在同一 control guard 复检。
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
        let mut reads = QueryReadBudget::new(budget, deadline)?;
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let scope = self.authorize_revision_with_budget(
            &reader,
            expected_scope,
            revision,
            principal,
            authorizer,
            &mut reads,
        )?;
        // 必需目标与消费者共用最初余额；准备失败同样进入真实范围的末段授权。
        let result = (|| {
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
            consumer(&reader, evidence.as_ref(), &mut reads)
        })();
        // 仅测试的读后同步点不持 control guard，不添加生产回调或共享请求状态。
        #[cfg(test)]
        crate::relation_request_tests::after_read(deadline);
        // 终检无法取得原控制库 guard 时拒绝全部结果，不在业务期限外等待另一个请求。
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        // 每轮能力回调后开始固定归属窗口；不续期结果读取或编码期限。
        let ownership = || {
            let authorization_deadline = Instant::now()
                .checked_add(std::time::Duration::from_millis(50))
                .ok_or(BusinessError::InvalidArgument)?;
            self.require_terminal_revision_ownership(
                revision,
                &scope,
                &control,
                authorization_deadline,
            )
        };
        let authorize = || {
            let timely = Self::observe_terminal_relation(&control, authorizer, principal, &scope)?;
            ownership()?;
            if !timely {
                return Err(EngineError::Business(BusinessError::BudgetExceeded));
            }
            Ok(())
        };
        authorize()?;
        let mut result = result?;
        let expired = Instant::now() >= deadline;
        let encoded = finish(&mut result, expired);
        // 本 Engine 的 guard 防止重入；独立连接仍可撤权，故编码后重新读实际 scope。
        authorize()?;
        encoded?;
        if !expired && Instant::now() >= deadline {
            let encoded = finish(&mut result, true);
            authorize()?;
            encoded?;
        }
        Ok(result)
    }

    /// 在调用方已有 SQL guard 下复核权限，不安装或清除另一个 deadline handler。
    /// 参数：control 为既有 guard，其他参数为当前真实身份；返回：授权或原错误。
    pub(super) fn require_terminal_relation(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        if control.scope_revoked(scope)? {
            return Err(BusinessError::PermissionDenied.into());
        }
        Self::require_with_control(
            control,
            authorizer,
            principal,
            &Permission::MetadataRead,
            scope,
        )?;
        if control.scope_revoked(scope)? {
            return Err(BusinessError::PermissionDenied.into());
        }
        Ok(())
    }

    /// 分阶段观察关系/历史末段权限，不得在既有 SQL deadline guard 中调用。
    /// 参数：control 为 Engine mutex guard，authorizer/principal/scope 为真实请求身份。
    /// 返回：授权允许是否及时；调用方须先复核其他侧和归属，再将迟到允许拒为预算失败。
    pub(super) fn observe_terminal_relation(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scope: &ScopeId,
    ) -> Result<bool, EngineError> {
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
        // 不在控制库 progress guard 内调用宿主授权器，避免嵌套或其耗时续期允许结果。
        let capability_deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let decision = authorizer.decide(principal, &Permission::MetadataRead, scope);
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
        // 不在此提前返回预算错误；调用方还须观察其他侧撤权及新鲜 revision 隔离。
        Ok(Instant::now() < capability_deadline)
    }

    /// 完成全部能力回调后，再纯读取每一侧的持久授权。
    /// 参数：control 为现有 guard，authorizer/principal/scopes 为同请求真实身份与范围。
    /// 返回：各侧均允许时的及时性；真实拒权优先，迟到允许由调用方完成归属观察后拒绝。
    pub(super) fn require_terminal_relations(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scopes: &[&ScopeId],
    ) -> Result<bool, EngineError> {
        let mut timely = true;
        for scope in scopes {
            timely &= Self::observe_terminal_relation(control, authorizer, principal, scope)?;
        }
        // 不再次调用能力授权器，避免最后一个回调继续使前侧复检失效。
        // guard 不冻结独立 SQLite 连接；此处是协作式末段观察边界。
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
        // 宿主回调不消耗后续新鲜 SQL 的窗口；迟到允许仍不能提交部分数据。
        let capability_deadline = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        let decisions = ownerships
            .iter()
            .map(|(_, scope)| authorizer.decide(principal, &Permission::MetadataRead, scope))
            .collect::<Vec<_>>();
        let after_callback = Instant::now()
            .checked_add(std::time::Duration::from_millis(50))
            .ok_or(BusinessError::InvalidArgument)?;
        control.with_read_deadline(after_callback, |control| {
            for ((_, scope), decision) in ownerships.iter().zip(decisions) {
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
        let reader =
            SqliteSnapshotStore::open_reader_until(&self.graph_path, after_callback, None)?;
        for (revision, (server, scope)) in revisions.iter().zip(&ownerships) {
            if !reader.revision_ownership_matches(revision, server, scope.as_str())? {
                return Err(BusinessError::PermissionDenied.into());
            }
        }
        if Instant::now() >= after_callback || Instant::now() >= capability_deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Instant::now() < deadline)
    }
}

/// 将控制 SQL 的期限、中断及锁等待映射为预算拒绝，其他原错误不改写。
fn terminal_control_error(error: EngineError) -> EngineError {
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
