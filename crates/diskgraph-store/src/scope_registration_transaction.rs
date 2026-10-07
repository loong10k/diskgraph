//! 注册的实时管理员权限、scope 与默认 grants 共用有限控制事务。
use crate::control_write_deadline::ControlWriteDeadline;
use crate::{ControlStore, Result, ScopeRecord};
use diskgraph_core::{Grant, Locator, Permission, PrincipalId, ScopeId, ServerId};
use rusqlite::params;
use std::time::Instant;

impl ControlStore {
    /// 注册前后核验持久管理员权限；None 拒绝，0 仅指从未发布策略的可信兼容模式。
    fn registration_admin_epoch(
        &self,
        principal: &PrincipalId,
        admin: &ScopeId,
    ) -> Result<Option<u64>> {
        match self.policy_state()? {
            None => Ok(Some(0)),
            Some((0, _)) | Some((_, true)) => Ok(None),
            Some((version, false)) => {
                let allowed: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3 AND policy_version=?4)",
                    params![principal.as_str(), Permission::ScopeAdmin.wire_name(), admin.as_str(), version as i64],
                    |row| row.get(0),
                )?;
                Ok(allowed.then_some(version))
            }
        }
    }

    /// 将范围和三项默认 grant 原子写入控制库；来源：原生 Rust SC-01/04。
    /// 参数：根/卷/主体/管理范围固定；deadline/expiry 不重建；reconcile 只写图库负向隔离。
    /// 返回：Some 为已提交范围，None 为实时拒权；提交前错误/unwind 回滚控制注册。
    /// RegistrationCommitted 明确携带已提交范围与后续连接清理错误，不能解释为未提交。
    /// 图拒绝先提交，控制 COMMIT 最后执行；控制回滚不解除图拒绝，不承诺跨库原子性。
    /// 请求能力须由 Engine 在调用前检查，事务内不运行可重入 Authorizer。
    #[allow(clippy::too_many_arguments)] // 根、主体、授权空间、原期限与跨库拒绝职责均独立必需。
    pub fn register_scope_with_grants_until(
        &mut self,
        root: &Locator,
        volume_id: Option<&str>,
        principal: &PrincipalId,
        admin: &ScopeId,
        deadline: Instant,
        expiry: Option<u64>,
        reconcile: impl FnOnce(&ServerId, &[ScopeRecord]) -> Result<()>,
    ) -> Result<Option<ScopeId>> {
        let window = ControlWriteDeadline::new(&self.connection, deadline, expiry)?;
        let result = (|| {
            window.begin()?;
            let Some(version) =
                window.statement(|| self.registration_admin_epoch(principal, admin))?
            else {
                return Ok(None);
            };
            let scope = window.statement(|| self.register_scope_on_connection(root, volume_id))?;
            let server = window.statement(|| self.existing_server_id())?;
            let scopes = window.statement(|| self.list_scopes())?;
            // 新 scope 尚未对其他连接可见；图库拒绝失败不能留下注册或部分 grants。
            reconcile(&server, &scopes)?;
            window.check()?;
            if version > 0 {
                for permission in [
                    Permission::IndexWrite,
                    Permission::MetadataRead,
                    Permission::OperationView,
                ] {
                    window.statement(|| {
                        self.upsert_grant_on_connection(&Grant {
                            principal: principal.clone(),
                            permission,
                            scope: scope.clone(),
                            policy_version: version,
                        })
                    })?;
                }
            }
            // 事务防止其他 writer 改 epoch；再拒绝同事务触发器造成的策略变化。
            if window.statement(|| self.registration_admin_epoch(principal, admin))?
                != Some(version)
            {
                return Ok(None);
            }
            window.commit()?;
            Ok(Some(scope))
        })();
        let committed = result
            .as_ref()
            .ok()
            .and_then(|scope| scope.as_ref())
            .cloned();
        match (committed, window.finish(result)) {
            (Some(scope_id), Err(source)) => Err(crate::StoreError::RegistrationCommitted {
                scope_id,
                source: Box::new(source),
            }),
            (_, other) => other,
        }
    }
}
