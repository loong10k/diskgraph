//! 关系请求复用真实 revision 授权与独立 reader，返回前在同一 control guard 复检。
use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};
use diskgraph_store::SqliteSnapshotStore;
use std::time::Instant;

impl Engine {
    /// 执行一次真实归属查询，有限完成结果后复核授权与期限。
    /// 参数：revision/身份、deadline、consumer 和 finish 只用于该请求。
    /// 返回：已复核结果或失败；真实格式错误不改写，预算失败仍终检授权。
    pub(super) fn with_relation_reader_until<T>(
        &self,
        revision: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
        consumer: impl FnOnce(&SqliteSnapshotStore, &str) -> Result<T, EngineError>,
        mut finish: impl FnMut(&mut T, bool) -> Result<(), EngineError>,
    ) -> Result<T, EngineError> {
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
        let scope =
            self.authorize_revision_with_reader(&reader, None, revision, principal, authorizer)?;
        let snapshot = reader.revision(revision)?.snapshot_id;
        let result = consumer(&reader, &snapshot);
        let budget_failure = matches!(
            &result,
            Err(EngineError::Business(
                BusinessError::BudgetExceeded | BusinessError::Timeout
            )) | Err(EngineError::Store(
                diskgraph_store::StoreError::BudgetExceeded
            ))
        );
        if result.is_err() && !budget_failure {
            return result;
        }
        // 仅测试的读后同步点不持 control guard，不添加生产回调或共享请求状态。
        #[cfg(test)]
        crate::relation_request_tests::after_read(deadline);
        let control = self.control_store()?;
        Self::require_terminal_relation(&control, authorizer, principal, &scope)?;
        let mut result = result?;
        let expired = Instant::now() >= deadline;
        finish(&mut result, expired)?;
        // 本 Engine 的 guard 防止重入；独立连接仍可撤权，故编码后重新读实际 scope。
        Self::require_terminal_relation(&control, authorizer, principal, &scope)?;
        if !expired && Instant::now() >= deadline {
            finish(&mut result, true)?;
            Self::require_terminal_relation(&control, authorizer, principal, &scope)?;
        }
        Ok(result)
    }

    /// 在既有 guard 下复核真实 scope 及授权器返回之后的撤销状态。
    /// 参数：control 为本 Engine guard，authorizer/principal/scope 为真实请求身份。
    /// 返回：末段授权结果；独立连接在 Authorizer 期间撤 scope 同样拒绝。
    pub(super) fn require_terminal_relation(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        if control.scope(scope)?.revoked {
            return Err(BusinessError::PermissionDenied.into());
        }
        Self::require_with_control(
            control,
            authorizer,
            principal,
            &Permission::MetadataRead,
            scope,
        )?;
        if control.scope(scope)?.revoked {
            return Err(BusinessError::PermissionDenied.into());
        }
        Ok(())
    }

    /// 完成全部能力回调后，再纯读取每一侧的持久授权。
    /// 参数：control 为现有 guard，authorizer/principal/scopes 为同请求真实身份与范围。
    /// 返回：各侧均仍允许；后侧回调撤前侧权限不能由顺序检查遗漏。
    pub(super) fn require_terminal_relations(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        scopes: &[&ScopeId],
    ) -> Result<(), EngineError> {
        for scope in scopes {
            Self::require_terminal_relation(control, authorizer, principal, scope)?;
        }
        // 不再次调用能力授权器，避免最后一个回调继续使前侧复检失效。
        // guard 不冻结独立 SQLite 连接；此处是协作式末段观察边界。
        for scope in scopes {
            if control.scope(scope)?.revoked
                || control.live_permission(principal, &Permission::MetadataRead, scope)?
                    == Some(false)
            {
                return Err(BusinessError::PermissionDenied.into());
            }
        }
        Ok(())
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
        // 末段授权必须可读实际归属；到期不能被用作跳过权限检查的理由。
        let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
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
        let mut control = self.control_store()?;
        let server_id = control.ensure_server()?;
        for (server, scope) in &ownerships {
            if server != server_id.as_str() || control.scope(scope)?.revoked {
                return Err(BusinessError::PermissionDenied.into());
            }
        }
        let scopes = ownerships
            .iter()
            .map(|(_, scope)| scope)
            .collect::<Vec<_>>();
        Self::require_terminal_relations(&control, authorizer, principal, &scopes)?;
        Ok(Instant::now() < deadline)
    }
}
