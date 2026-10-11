//! 成组revision在响应编码后的实时授权终检；保留原接口、期限和撤权优先级。
use crate::authority_expiry::check_authority_expiry;
use crate::relation_request::terminal_control_error;
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};
use diskgraph_store::{RevisionOwnershipReader, SqliteSnapshotStore};
use std::time::Instant;

impl Engine {
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
            .checked_add(crate::terminal_authorization_windows::DATABASE_WINDOW)
            .ok_or(BusinessError::InvalidArgument)?;
        let reader = crate::authorization_phase_diagnostic::observe("grouped_reader_open", || {
            SqliteSnapshotStore::open_reader_until(&self.graph_path, observation_deadline, None)
                .map_err(|error| terminal_control_error(error.into()))
        })?;
        let ownerships =
            crate::authorization_phase_diagnostic::observe("grouped_ownership_sql", || {
                revisions
                    .iter()
                    .map(|revision| {
                        let (server, scope) = reader
                            .revision_ownership(revision)?
                            .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
                        let scope = ScopeId::new(scope)
                            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
                        Ok((server, scope))
                    })
                    .collect::<Result<Vec<_>, EngineError>>()
                    .map_err(terminal_control_error)
            })?;
        let control = self
            .try_control_store()?
            .ok_or(EngineError::Business(BusinessError::BudgetExceeded))?;
        check_authority_expiry(expiry)?;
        let (withdrawals, generation) =
            crate::authorization_phase_diagnostic::observe("grouped_initial_control", || {
                control
                    .with_read_deadline(observation_deadline, |control| {
                        let server_id = control.existing_server_id()?;
                        for (server, scope) in &ownerships {
                            if server != server_id.as_str() || control.scope_revoked(scope)? {
                                return Err(EngineError::Business(BusinessError::PermissionDenied));
                            }
                        }
                        let withdrawals = ownerships
                    .iter()
                    .map(|(_, scope)| {
                        crate::request_withdrawal_witness::RequestWithdrawalWitness::capture(
                            control, principal, scope,
                        )
                    })
                    .collect::<Result<Vec<_>, EngineError>>()?;
                        Ok::<_, EngineError>((withdrawals, control.authorization_generation()?))
                    })
                    .map_err(terminal_control_error)
            })?;
        drop(reader);
        drop(control);
        check_authority_expiry(expiry)?;
        // 宿主回调不消耗后续新鲜 SQL 的窗口；迟到允许仍不能提交部分数据。
        let capability_deadline = Instant::now()
            .checked_add(crate::terminal_authorization_windows::CAPABILITY_WINDOW)
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
        let after_callback = Instant::now()
            .checked_add(crate::terminal_authorization_windows::DATABASE_WINDOW)
            .ok_or(BusinessError::InvalidArgument)?;
        let bound_revisions: Vec<_> = revisions
            .iter()
            .zip(&ownerships)
            .map(|(revision, (_, scope))| (*revision, scope))
            .collect();
        // 负向撤权仍优先；未知代次冲突须先观察实际隔离，不用冲突遮盖已知拒权。
        for withdrawal in &withdrawals {
            self.prioritize_revision_quarantine(
                withdrawal.check(&control),
                &bound_revisions,
                &control,
                after_callback,
            )?;
        }
        check_authority_expiry(expiry)?;
        let terminal_observation =
            crate::authorization_phase_diagnostic::observe("grouped_terminal_observation", || {
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
                            || control.live_permission(
                                principal,
                                &Permission::MetadataRead,
                                scope,
                            )? == Some(false)
                        {
                            return Err(EngineError::Business(BusinessError::PermissionDenied));
                        }
                    }
                    for withdrawal in &withdrawals {
                        withdrawal.check(control)?;
                        if !withdrawal.has_native_watch()
                            && control.authorization_generation()? != generation
                        {
                            return Err(BusinessError::Conflict.into());
                        }
                    }
                    Ok::<(), EngineError>(())
                })?;
                check_authority_expiry(expiry)?;
                // 归属末检不需要节点查询的 cache/temp PRAGMA；仍新开连接观察当前 WAL。
                // 固定比较首次捕获的 server 和 scope，不能接受终检间隙被替换的绑定。
                let reader = RevisionOwnershipReader::open_until(&self.graph_path, after_callback)?;
                for (revision, (server, scope)) in revisions.iter().zip(&ownerships) {
                    if !reader.matches(revision, server, scope.as_str())? {
                        return Err(BusinessError::PermissionDenied.into());
                    }
                }
                Ok::<(), EngineError>(())
            });
        // 独立控制连接可在终态读取中提交撤权，已知拒权不能被存储超时遮盖。
        for withdrawal in &withdrawals {
            self.prioritize_revision_quarantine(
                withdrawal.check(&control),
                &bound_revisions,
                &control,
                after_callback,
            )?;
        }
        self.prioritize_revision_quarantine(
            terminal_observation.map_err(terminal_control_error),
            &bound_revisions,
            &control,
            after_callback,
        )?;
        check_authority_expiry(expiry)?;
        if Instant::now() >= after_callback || !capability_timely {
            return crate::authorization_phase_diagnostic::observe(
                "grouped_terminal_deadline",
                || Err(BusinessError::BudgetExceeded.into()),
            );
        }
        check_authority_expiry(expiry)?;
        Ok(Instant::now() < deadline)
    }
}
