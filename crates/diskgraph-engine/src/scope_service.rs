//! 共享 Engine 的 scope_service 职责；原调用与持锁顺序保持。

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
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(5))
            .ok_or(BusinessError::InvalidArgument)?;
        let expiry = authorizer.expires_at_unix_seconds();
        self.require(
            authorizer,
            principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        )?;
        let canonical = root.canonicalize()?;
        let locator = Locator::from_native_path(&canonical);
        let volume_id = locator_volume_id(&locator);
        // 与扫描发布/历史回收保持 graph→control 顺序，注册与隔离完成前不授新权限。
        let mut graph = self.graph()?;
        let mut control = self.control()?;
        // 等待路径解析和双锁期间，能力或数据库策略可能失效；写入前复核。
        Self::require_with_control(
            &control,
            authorizer,
            principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        )?;
        let registered = control.register_scope_with_grants_until(
            &locator,
            volume_id.as_deref(),
            principal,
            &admin_scope(),
            deadline,
            expiry,
            |server, scopes| {
                crate::revision_root_reconciliation::eligible_roots(&mut graph, server, scopes)
                    .map(|_| ())
            },
        );
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
        let control = self.control()?;
        let permitted = |scope: &ScopeId| -> Result<bool, EngineError> {
            match Self::require_with_control(
                &control,
                authorizer,
                principal,
                &Permission::MetadataRead,
                scope,
            ) {
                Ok(()) => Ok(true),
                Err(EngineError::Business(BusinessError::PermissionDenied)) => Ok(false),
                Err(error) => Err(error),
            }
        };
        let authorizer_is_permissive = permitted(&admin_scope())?;
        let scopes = control.list_scopes()?;
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let mut allowed = Vec::new();
        for scope in scopes {
            if permitted(&scope.scope_id)? {
                allowed.push(scope);
            }
        }
        // A principal whose only grant is administration sees the whole
        // registry; every other principal sees only its own scopes.
        if allowed.is_empty() && authorizer_is_permissive {
            return Ok(control.list_scopes()?);
        }
        Ok(allowed)
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
