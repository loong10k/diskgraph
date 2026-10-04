//! CLI authorization 的真实职责实现。
use diskgraph_core::{Authorizer, BusinessError, Permission, PrincipalId, ScopeId};
use diskgraph_engine::EngineError;

/// 保留 require_metadata 的原生业务职责与错误语义。来源：DiskGraph CLI main::require_metadata。
/// 参数：与原入口的 require_metadata 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn require_metadata(
    authorizer: &dyn Authorizer,
    principal: &PrincipalId,
    scope_id: &ScopeId,
) -> Result<(), EngineError> {
    match authorizer.decide(principal, &Permission::MetadataRead, scope_id) {
        diskgraph_core::Decision::Allowed => Ok(()),
        diskgraph_core::Decision::Denied(_) => {
            Err(EngineError::Business(BusinessError::PermissionDenied))
        }
    }
}
