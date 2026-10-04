//! 请求身份固定与实时授权；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::{McpService, auth, request_authorizer, request_context};
use diskgraph_core::Authorizer;
use diskgraph_engine::{EngineError, admin_scope};

impl McpService {
    /// 从当前控制库重建实时授权器，使启动后新增范围和授权即时生效。
    /// 参数：无。返回：包含请求能力上限和期限的授权器或错误。
    pub(crate) fn authorizer(&self) -> Result<request_authorizer::RequestAuthorizer, EngineError> {
        Ok(request_authorizer::RequestAuthorizer {
            policy: self.engine.policy_authorizer()?,
            capabilities: self.context.capabilities(),
            expires_at: self.context.expires_at(),
        })
    }

    /// 为已认证主体创建独立 HTTP 请求状态，共享 Engine，不覆盖本地主体。
    /// 参数：identity 为已校验身份。返回：该主体的新请求服务。
    pub fn for_identity(&self, identity: &auth::AuthenticatedPrincipal) -> Self {
        self.for_transport_identity(identity, "http")
    }

    /// 将已认证身份固定到传输，请求状态不写入共享 Engine。
    /// 参数：identity 为已校验身份，transport 为服务端传输名称。返回：独立请求服务。
    pub(crate) fn for_transport_identity(
        &self,
        identity: &auth::AuthenticatedPrincipal,
        transport: &'static str,
    ) -> Self {
        let mut request = self.clone();
        request.context = request_context::RequestContext::authenticated(identity, transport);
        request
    }

    /// 查询认证主体是否仍有实际数据库授权，供长连接撤权终止。
    /// 参数：identity 为已认证主体。返回：存在有效实时授权时为 true。
    pub(crate) fn identity_is_live(&self, identity: &auth::AuthenticatedPrincipal) -> bool {
        if identity.permissions.is_empty() {
            return false;
        }
        let request = self.for_identity(identity);
        let Ok(policy) = request.authorizer() else {
            return false;
        };
        if identity.permissions.iter().any(|permission| {
            matches!(
                policy.decide(&identity.principal, permission, &admin_scope()),
                diskgraph_core::Decision::Allowed
            )
        }) {
            return true;
        }
        self.engine
            .control_store()
            .ok()
            .and_then(|store| store.list_scopes().ok())
            .is_some_and(|scopes| {
                scopes.iter().any(|scope| {
                    !scope.revoked
                        && identity.permissions.iter().any(|permission| {
                            matches!(
                                policy.decide(&identity.principal, permission, &scope.scope_id),
                                diskgraph_core::Decision::Allowed
                            )
                        })
                })
            })
    }

    /// 读取请求传输和经过校验的签发方，供日志使用。
    /// 参数：无。返回：传输名称及可选签发方借用。
    pub fn request_transport(&self) -> (&'static str, Option<&str>) {
        (self.context.transport(), self.context.issuer())
    }
}
