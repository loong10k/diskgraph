//! 实际范围与版本权限绑定；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::McpService;
use diskgraph_core::{Authorizer, BusinessError, Permission, ScopeId};
use diskgraph_engine::EngineError;
use serde_json::Value;

impl McpService {
    /// 解析可选范围参数；省略时沿现有列表顺序选择未撤销范围。
    /// 参数：arguments 为可选 scope 字段。返回：可选范围；未知范围返回 not_found，不伪装 permission_denied。
    pub(crate) fn resolve_scope_until(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Option<ScopeId>, EngineError> {
        if let Some(scope) = arguments.get("scope").and_then(Value::as_str) {
            let scope_id = ScopeId::new(scope.to_owned())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // Unknown scopes are not_found, not permission_denied: the caller
            // can tell a wrong scope from a hidden one.
            match self.engine.scope_until(&scope_id, deadline) {
                Ok(_) => {}
                Err(EngineError::Store(diskgraph_store::StoreError::ScopeNotFound(_))) => {
                    return Err(EngineError::Business(BusinessError::NotFound));
                }
                Err(error) => return Err(error),
            }
            return Ok(Some(scope_id));
        }
        let scopes = self.engine.list_scopes_until(
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
        )?;
        Ok(scopes
            .into_iter()
            .find(|scope| !scope.revoked)
            .map(|scope| scope.scope_id))
    }

    /// 要求当前请求已有明确范围。
    /// 参数：scope 为可选范围。返回：实际范围或参数错误。
    pub(crate) fn require_scope(&self, scope: &Option<ScopeId>) -> Result<ScopeId, EngineError> {
        scope
            .clone()
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))
    }

    /// 固定显式版本或范围 latest，并复验实际版本所有者授权。
    /// 参数：scope 为范围断言，arguments 为版本字段。返回：授权版本标识或错误。
    pub(crate) fn require_revision(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<String, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = match arguments.get("revision").and_then(Value::as_str) {
            Some(revision) => revision.to_owned(),
            None => self
                .engine
                .latest_revision(&scope_id)?
                .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
        };
        self.engine.authorize_revision(
            Some(&scope_id),
            &revision,
            self.context.principal(),
            &self.authorizer()?,
        )?;
        Ok(revision)
    }

    /// 在原请求期限内固定版本并验证实际归属，预算错误不会映射为拒绝或不存在。
    /// 参数：scope/arguments 为版本断言，deadline 为原期限。返回：授权版本或错误。
    pub(crate) fn require_revision_until(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<String, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let revision = match arguments.get("revision").and_then(Value::as_str) {
            Some(revision) => revision.to_owned(),
            None => self
                .engine
                .latest_revision_until(&scope_id, deadline)?
                .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
        };
        self.engine.authorize_revision_until(
            Some(&scope_id),
            &revision,
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
        )?;
        Ok(revision)
    }

    /// 在原请求期限内检查 token 能力与实时数据库策略的交集。
    /// 参数：permission/scope 为授权目标，deadline 为原期限。返回：允许或预算/拒绝错误。
    pub(crate) fn require_until(
        &self,
        permission: &Permission,
        scope: &ScopeId,
        deadline: std::time::Instant,
    ) -> Result<(), EngineError> {
        let authorizer = self.authorizer_until(deadline)?;
        let decision = authorizer.decide(self.context.principal(), permission, scope);
        match decision {
            diskgraph_core::Decision::Allowed => {
                if std::time::Instant::now() >= deadline {
                    return Err(EngineError::Business(BusinessError::BudgetExceeded));
                }
                Ok(())
            }
            diskgraph_core::Decision::Denied(_) => {
                Err(EngineError::Business(BusinessError::PermissionDenied))
            }
        }
    }

    /// 使用实时策略与当前请求身份检查指定权限。
    /// 参数：permission 为能力，scope 为实际授权范围。返回：允许时为空结果，否则 permission_denied。
    pub(crate) fn require(
        &self,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        let authorizer = self
            .authorizer()
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
        match authorizer.decide(self.context.principal(), permission, scope) {
            diskgraph_core::Decision::Allowed => Ok(()),
            diskgraph_core::Decision::Denied(_) => {
                Err(EngineError::Business(BusinessError::PermissionDenied))
            }
        }
    }
}
