use crate::EngineError;
use diskgraph_core::{BusinessError, Permission, PrincipalId, ScopeId};
use diskgraph_store::{AuthorizationWithdrawalStatus, AuthorizationWithdrawalWatch, ControlStore};

/// 固定请求依赖的负向撤权见证；未知平台仍执行实时数据库授权。
/// 来源：DiskGraph 原生 Rust D45；没有 Java 对应对象。
pub(super) struct RequestWithdrawalWitness {
    watch: Option<AuthorizationWithdrawalWatch>,
}

impl RequestWithdrawalWitness {
    /// 在首次授权前注册真实主体和 scope 的元数据读取依赖。
    /// 参数：control 是请求原控制连接，principal/scope 是已解析的实际归属。
    /// 返回：请求见证或原存储错误；此方法本身不能授予权限。
    pub(super) fn capture(
        control: &ControlStore,
        principal: &PrincipalId,
        scope: &ScopeId,
    ) -> Result<Self, EngineError> {
        Self::capture_permission(control, principal, scope, &Permission::MetadataRead)
    }

    /// 为实际内容或元数据权限注册负向见证。
    /// 参数：control 为原连接，主体/范围/权限为原请求依赖；返回：不能授予权限的见证。
    pub(super) fn capture_permission(
        control: &ControlStore,
        principal: &PrincipalId,
        scope: &ScopeId,
        permission: &Permission,
    ) -> Result<Self, EngineError> {
        Ok(Self {
            watch: control.watch_authorization_withdrawal(principal, scope, permission)?,
        })
    }

    /// 查询当前见证是否有可靠原生负向通知能力。
    /// 参数：无；返回：true 表示存在原连接绑定通知，false 须由调用方保守处理未知变化。
    pub(super) fn has_native_watch(&self) -> bool {
        self.watch.is_some()
    }

    /// 检查已提交撤权，不执行 SQL，也不替代当前授权与原期限。
    /// 参数：control 必须是首次注册使用的控制连接。
    /// 返回：已知撤权为拒权，连接代次失效为冲突，否则继续实时授权。
    pub(super) fn check(&self, control: &ControlStore) -> Result<(), EngineError> {
        match self.watch.as_ref().map(|watch| watch.status_for(control)) {
            Some(AuthorizationWithdrawalStatus::Withdrawn) => {
                Err(BusinessError::PermissionDenied.into())
            }
            Some(AuthorizationWithdrawalStatus::Invalidated) => Err(BusinessError::Conflict.into()),
            Some(AuthorizationWithdrawalStatus::Unchanged) | None => Ok(()),
        }
    }
}
