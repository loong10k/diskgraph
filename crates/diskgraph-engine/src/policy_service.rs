//! 共享 Engine 的 policy_service 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, Grant, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};

impl Engine {
    /// 从控制库重建实时策略授权器。
    /// 参数：无。
    /// 返回：授权器或控制库失败。
    /// The live policy authorizer rebuilt from the control store.
    pub fn policy_authorizer(&self) -> Result<PolicyAuthorizer, EngineError> {
        Ok(self.control()?.authorizer()?)
    }
}

impl Engine {
    /// 可信本机初始化管理授权并保留已撤策略状态。
    /// 参数：principal 为可信本机管理主体。
    /// 返回：初始化成功或持久化失败；不授予正文权限。
    /// One-time local bootstrap (single-user CLI/service mode): publishes
    /// policy v1 if absent and grants this principal server administration,
    /// index management, metadata reads, and operation views on the admin
    /// scope. Authorization for everything else stays default-deny.
    pub fn bootstrap_local_admin(&self, principal: &PrincipalId) -> Result<(), EngineError> {
        let mut control = self.control()?;
        // Publish the initial version only when nothing was ever published:
        // a revoked policy must stay revoked until an explicit republish.
        if control.policy_state()?.is_none() {
            control.publish_policy_version(1)?;
        }
        let version = control.policy_version()?;
        for permission in [
            Permission::ScopeAdmin,
            Permission::IndexWrite,
            Permission::MetadataRead,
            Permission::OperationView,
        ] {
            control.upsert_grant(&Grant {
                principal: principal.clone(),
                permission,
                scope: admin_scope(),
                policy_version: version,
            })?;
        }
        // Renew every existing grant into the current epoch, so a policy bump
        // does not silently strip scope-local rights the administrator already
        // issued; explicit revocation is what takes rights away (SC-04). A
        // revoked policy is left untouched: renewal must not resurrect it.
        if let Some((_, true)) = control.policy_state()? {
            return Ok(());
        }
        for grant in control.all_grants()? {
            control.upsert_grant(&Grant {
                policy_version: version,
                ..grant
            })?;
        }
        Ok(())
    }
}

impl Engine {
    /// 可信管理入口添加或撤销主体的范围正文授权。
    /// 参数：scope_id/principal 为授权对象，allow 决定添加或撤销。
    /// 返回：更新成功或无策略/控制库失败；调用方负责管理入口授权。
    /// Grants or withdraws the right to read file contents inside one scope.
    ///
    /// Registering a scope hands out index, metadata and view rights and
    /// nothing more, because reading a file's bytes is a different question
    /// from reading its size (D13). A caller that wants to verify a
    /// comparison against contents asks for this first, and can take it back;
    /// a grant nobody asked for is exactly the kind that outlives its reason.
    pub fn set_content_read(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        allow: bool,
    ) -> Result<(), EngineError> {
        let mut control = self.control_store()?;
        let version = control.policy_version()?;
        if version == 0 {
            // No policy epoch means no grants exist to add one to.
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if allow {
            control.upsert_grant(&Grant {
                principal: principal.clone(),
                permission: Permission::ContentRead,
                scope: scope_id.clone(),
                policy_version: version,
            })?;
        } else {
            control.revoke_grant(principal, &Permission::ContentRead, scope_id)?;
        }
        Ok(())
    }
}

impl Engine {
    /// 通过持久管理 grant 重发策略并续入既有授权。
    /// 参数：version 为新版本，principal 为管理主体；authorizer 保留兼容参数。
    /// 返回：更新成功或缺管理授权/存储失败。
    /// Publishes a new policy version; grants from older versions stop
    /// applying and every cursor issued under them is refused (SC-04, P4-5.9).
    pub fn publish_policy_version(
        &self,
        version: u64,
        principal: &PrincipalId,
        _authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        // Management validates against the durable admin grant, not the live
        // authorizer: a revoked policy must remain republishable (SC-04).
        if !self.control()?.holds_admin(principal)? {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let mut control = self.control()?;
        control.publish_policy_version(version)?;
        // Existing grants carry into the new epoch unless explicitly revoked:
        // a version bump expires forged/stale artifacts (cursors), not the
        // administrator's standing grants (SC-04).
        for grant in control.all_grants()? {
            control.upsert_grant(&Grant {
                policy_version: version,
                ..grant
            })?;
        }
        Ok(())
    }
}

impl Engine {
    /// 通过持久管理 grant 撤销整个策略。
    /// 参数：principal 为管理主体；authorizer 保留兼容参数。
    /// 返回：撤销成功或缺管理授权/存储失败。
    /// Revokes the whole policy; nothing is authorized until republished.
    pub fn revoke_policy(
        &self,
        principal: &PrincipalId,
        _authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        if !self.control()?.holds_admin(principal)? {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        self.control()?.revoke_policy()?;
        Ok(())
    }
}

/// 返回服务端管理权限使用的固定 scope。
/// 参数：无。
/// 返回：diskgraph-admin 范围标识。
/// The scope that authorizes server administration (scope add/remove, serve).
pub fn admin_scope() -> ScopeId {
    ScopeId::new("diskgraph-admin").expect("constant is valid")
}
