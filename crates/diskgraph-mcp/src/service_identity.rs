//! 请求身份固定与实时授权；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::{McpService, auth, request_authorizer, request_context};
use diskgraph_engine::EngineError;

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

    /// 在原请求期限内构建带能力上限与 token 到期的实时授权器。
    /// 参数：deadline 为协议分发前建立的同一截止时间。
    /// 返回：请求授权器或有界控制锁/SQL 错误；不延长 token 有效期。
    pub(crate) fn authorizer_until(
        &self,
        deadline: std::time::Instant,
    ) -> Result<request_authorizer::RequestAuthorizer, EngineError> {
        Ok(request_authorizer::RequestAuthorizer {
            policy: self
                .engine
                .policy_authorizer_for_principal_until(self.context.principal(), deadline)?,
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
        self.identity_liveness(identity).unwrap_or(false)
    }

    /// 有界确认握手主体实时权限。参数：已认证身份；返回：允许、拒绝或无法确认的原错误。
    /// 与存活检查共用原窗口；握手在错误时返回不可用，不签发成功会话。
    pub(crate) fn identity_liveness(
        &self,
        identity: &auth::AuthenticatedPrincipal,
    ) -> Result<bool, EngineError> {
        if identity.permissions.is_empty() {
            return Ok(false);
        }
        // 每轮控制观察共用固定窗口；失败关闭，不等待无限期mutex或缓存grant。
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
        let unexpired = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .is_ok_and(|now| now.as_secs() < identity.expires_at_unix_seconds)
        };
        if !unexpired() {
            return Ok(false);
        }
        let live = self.engine.has_live_permission_until(
            &identity.principal,
            &identity.permissions,
            deadline,
        )?;
        Ok(live && unexpired())
    }

    /// 读取请求传输和经过校验的签发方，供日志使用。
    /// 参数：无。返回：传输名称及可选签发方借用。
    pub fn request_transport(&self) -> (&'static str, Option<&str>) {
        (self.context.transport(), self.context.issuer())
    }
}
