//! scope 注册、可见范围列表与撤销服务；列表能力回调位于控制锁外。

use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{
    Authorizer, BusinessError, Locator, Permission, PrincipalId, ResourceRef, ScopeId,
};
use diskgraph_store::{ScopeRecord, StoreError};
use std::path::Path;

impl Engine {
    /// 以管理权限注册规范根并授予注册主体既有 scope 权限。
    /// 参数：root 为本机根；principal/authorizer 为主体与请求能力。
    /// 返回：幂等 scope ID 或授权/定位/持久化失败。
    /// Registers or idempotently returns a scope for a native root (SC-01).
    pub fn register_scope(
        &self,
        root: &Path,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        let started = std::time::Instant::now();
        let diagnostic = std::env::var_os("DISKGRAPH_SCAN_DIAGNOSTICS").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        // 仅固定阶段和单调耗时；不输出根路径、身份或凭据，不重置原期限。
        let trace = |phase: &str| {
            if diagnostic {
                eprintln!(
                    "diskgraph: scope_registration_phase={phase} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
            }
        };
        trace("begin");
        let deadline = started
            .checked_add(std::time::Duration::from_secs(5))
            .ok_or(BusinessError::InvalidArgument)?;
        let expiry = authorizer.expires_at_unix_seconds();
        let initial_authorization = (|| {
            let decision = authorizer.decide(principal, &Permission::ScopeAdmin, &admin_scope());
            crate::authority_expiry::check_authority_expiry(expiry)?;
            let control = self.control_until(deadline)?;
            crate::authority_expiry::check_authority_expiry(expiry)?;
            control.with_read_deadline(deadline, |control| {
                Self::require_decision_with_control(
                    control,
                    decision,
                    principal,
                    &Permission::ScopeAdmin,
                    &admin_scope(),
                )
            })?;
            crate::authority_expiry::check_authority_expiry(expiry)
        })();
        initial_authorization.inspect_err(|_| trace("initial_authorization_failed"))?;
        trace("initial_authorization_complete");
        let canonical = root
            .canonicalize()
            .inspect_err(|_| trace("canonicalization_failed"))?;
        trace("canonicalization_complete");
        let locator = Locator::from_native_path(&canonical);
        let volume_id = locator_volume_id(&locator);
        // 路径解析后的第二次能力观察在双锁外；写入前仍在锁内核验当前持久授权。
        let decision = authorizer.decide(principal, &Permission::ScopeAdmin, &admin_scope());
        // 与扫描发布/历史回收保持 graph→control 顺序，注册与隔离完成前不授新权限。
        let mut graph = self.graph_until(deadline)?;
        let mut control = self.control_until(deadline)?;
        trace("locks_acquired");
        // 等待双锁期间的撤权必须拒绝；固定 token expiry 由原写守卫继续核验。
        control.with_read_deadline(deadline, |control| {
            Self::require_decision_with_control(
                control,
                decision,
                principal,
                &Permission::ScopeAdmin,
                &admin_scope(),
            )
        })?;
        let registered = control.register_scope_with_grants_until(
            &locator,
            volume_id.as_deref(),
            principal,
            &admin_scope(),
            deadline,
            expiry,
            |server, scopes| {
                trace("reconciliation_begin");
                let result =
                    crate::revision_root_reconciliation::eligible_roots(&mut graph, server, scopes)
                        .map(|_| ());
                trace(if result.is_ok() {
                    "reconciliation_complete"
                } else {
                    "reconciliation_failed"
                });
                result
            },
        );
        trace(if registered.is_ok() {
            "transaction_returned"
        } else {
            "transaction_failed"
        });
        match registered {
            Ok(Some(scope)) => Ok(scope),
            Ok(None) => Err(BusinessError::PermissionDenied.into()),
            // 只转换写守卫确认的原认证到期；已提交清理错误不伪装成拒权。
            Err(StoreError::Conflict(message)) if message == "job request authority expired" => {
                Err(BusinessError::PermissionDenied.into())
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Engine {
    /// 按实时元数据权限筛选注册范围并保留既有管理回退。
    /// 参数：principal/authorizer 为主体与请求能力。
    /// 返回：可见范围列表或查询失败。
    /// Lists registered scopes the principal may read metadata for. Listing is
    /// a metadata read scoped to each entry: the registry is visible exactly
    /// as far as the caller's grants reach, and an ungranted principal sees
    /// nothing at all.
    pub fn list_scopes(
        &self,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<ScopeRecord>, EngineError> {
        self.list_scopes_with_deadline(principal, authorizer, None)
    }

    /// 在原请求期限内列出实时可见范围，控制锁及 SQL 共用预算。
    /// 参数：principal/authorizer 为请求身份，deadline 为原截止时间。返回：可见范围或预算/授权错误。
    pub fn list_scopes_until(
        &self,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: std::time::Instant,
    ) -> Result<Vec<ScopeRecord>, EngineError> {
        self.list_scopes_with_deadline(principal, authorizer, Some(deadline))
    }

    fn read_scope_control<T>(
        &self,
        deadline: Option<std::time::Instant>,
        read: impl FnOnce(&diskgraph_store::ControlStore) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        match deadline {
            Some(deadline) => self
                .control_until(deadline)?
                .with_read_deadline(deadline, read),
            None => read(&*self.control()?),
        }
    }

    fn list_scopes_with_deadline(
        &self,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Option<std::time::Instant>,
    ) -> Result<Vec<ScopeRecord>, EngineError> {
        let admin_decision =
            authorizer.decide(principal, &Permission::MetadataRead, &admin_scope());
        let scopes = self.read_scope_control(deadline, |control| Ok(control.list_scopes()?))?;
        // 外部回调不能持有共享锁；收集决定后再读取实时持久授权，覆盖跨项撤权。
        let decisions = scopes
            .into_iter()
            .map(|scope| {
                let decision =
                    authorizer.decide(principal, &Permission::MetadataRead, &scope.scope_id);
                (scope, decision)
            })
            .collect::<Vec<_>>();
        let expiry = authorizer.expires_at_unix_seconds();
        crate::authority_expiry::check_authority_expiry(expiry)?;
        self.read_scope_control(deadline, |control| {
            let permitted = |decision, scope: &ScopeId| -> Result<bool, EngineError> {
                match Self::require_decision_with_control(
                    control,
                    decision,
                    principal,
                    &Permission::MetadataRead,
                    scope,
                ) {
                    Ok(()) => Ok(true),
                    Err(EngineError::Business(BusinessError::PermissionDenied)) => Ok(false),
                    Err(error) => Err(error),
                }
            };
            let authorizer_is_permissive = permitted(admin_decision, &admin_scope())?;
            let mut allowed = Vec::new();
            for (scope, decision) in decisions {
                if permitted(decision, &scope.scope_id)? {
                    allowed.push(scope);
                }
            }
            // 管理回退使用所有回调完成后的持久授权；不能复用撤权之前的允许状态。
            if allowed.is_empty() && authorizer_is_permissive {
                allowed = control.list_scopes()?;
            }
            crate::authority_expiry::check_authority_expiry(expiry)?;
            Ok(allowed)
        })
    }
}

impl Engine {
    /// 撤销注册范围，保留用户文件与历史记录。
    /// 参数：scope_id 为待撤范围；principal/authorizer 须有管理权限。
    /// 返回：撤销成功或授权/控制库失败。
    /// Revokes a scope; lookups and new jobs stop (SC-04). Files, snapshots,
    /// and control history are never deleted.
    pub fn revoke_scope(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.require(
            authorizer,
            principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        )?;
        self.control()?.revoke_scope(scope_id)?;
        Ok(())
    }
}

impl Engine {
    /// 可信内部读取注册范围，不独立授权请求。
    /// 参数：scope_id 为持久范围标识。
    /// 返回：范围记录或查询失败。
    /// Loads one scope.
    pub fn scope(&self, scope_id: &ScopeId) -> Result<ScopeRecord, EngineError> {
        Ok(self.control()?.scope(scope_id)?)
    }

    /// 在原请求期限内读取注册范围，不替代调用方的权限检查。
    /// 参数：scope_id 为持久范围，deadline 为原截止时间。返回：记录或预算/存储错误。
    pub fn scope_until(
        &self,
        scope_id: &ScopeId,
        deadline: std::time::Instant,
    ) -> Result<ScopeRecord, EngineError> {
        self.read_scope_control(Some(deadline), |control| Ok(control.scope(scope_id)?))
    }
}

impl Engine {
    /// 可信组装完整资源引用，标识本身不是授权令牌。
    /// 参数：scope/revision/node 由调用方核验归属。
    /// 返回：资源引用或非法 revision/控制库失败。
    /// Full reference for one node inside a published revision (SC-02 shape).
    pub fn resource_ref(
        &self,
        scope_id: &ScopeId,
        revision_id: &str,
        node_id: u64,
    ) -> Result<ResourceRef, EngineError> {
        Ok(ResourceRef {
            server_id: self.server_id()?,
            scope_id: scope_id.clone(),
            revision_id: diskgraph_core::RevisionId::new(revision_id.to_owned())
                .map_err(|error| EngineError::Store(StoreError::InvalidGraph(error.to_string())))?,
            node_id,
        })
    }
}

#[cfg(unix)]
fn locator_volume_id(locator: &Locator) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let path = locator.to_native_path().ok()?;
    std::fs::metadata(path).ok().map(|m| m.dev().to_string())
}
#[cfg(windows)]
fn locator_volume_id(locator: &Locator) -> Option<String> {
    let path = locator.to_native_path().ok()?;
    diskgraph_disktree_core::space::device_for(&path)
}
#[cfg(not(any(unix, windows)))]
fn locator_volume_id(_locator: &Locator) -> Option<String> {
    None
}
