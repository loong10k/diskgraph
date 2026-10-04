use diskgraph_core::{BusinessError, JobRequestAuthority, Permission, PrincipalId};

/// 不可变的请求身份；与共享 Engine 分离，由认证层一次性构造。
#[derive(Clone)]
pub(crate) struct RequestContext {
    principal: PrincipalId,
    capabilities: Option<Vec<Permission>>,
    transport: &'static str,
    issuer: Option<String>,
    expires_at: Option<u64>,
}

impl RequestContext {
    /// 构造可信本地身份。
    pub(crate) fn local(principal: PrincipalId) -> Self {
        Self {
            principal,
            capabilities: None,
            transport: "stdio",
            issuer: None,
            expires_at: None,
        }
    }

    /// 远程服务初始身份没有能力，不得继承已有本地数据库授权。
    pub(crate) fn unauthenticated(principal: PrincipalId) -> Self {
        Self {
            principal,
            capabilities: Some(Vec::new()),
            transport: "unauthenticated-http",
            issuer: None,
            expires_at: None,
        }
    }

    pub(crate) fn trusted_local(&self) -> bool {
        self.capabilities.is_none()
    }

    /// 构造通过 token 校验的远程身份，能力只允许缩小数据库授权。
    pub(crate) fn authenticated(
        identity: &crate::auth::AuthenticatedPrincipal,
        transport: &'static str,
    ) -> Self {
        Self {
            principal: identity.principal.clone(),
            capabilities: Some(identity.permissions.clone()),
            transport,
            issuer: Some(identity.issuer.clone()),
            expires_at: Some(identity.expires_at_unix_seconds),
        }
    }

    /// 将已验证请求身份转成持久任务来源；不读取工具 JSON 参数。
    /// 参数：无，使用私有不可变上下文；返回：可信本机/已认证远程身份或拒绝未认证请求。
    pub(crate) fn job_authority(&self) -> Result<JobRequestAuthority, BusinessError> {
        if self.trusted_local() {
            JobRequestAuthority::trusted_local(self.principal.clone(), self.transport)
        } else {
            JobRequestAuthority::authenticated_remote(
                self.principal.clone(),
                self.issuer.clone().ok_or(BusinessError::PermissionDenied)?,
                self.transport,
                self.capabilities
                    .clone()
                    .ok_or(BusinessError::PermissionDenied)?,
                self.expires_at.ok_or(BusinessError::PermissionDenied)?,
            )
        }
    }

    pub(crate) fn expires_at(&self) -> Option<u64> {
        self.expires_at
    }

    pub(crate) fn principal(&self) -> &PrincipalId {
        &self.principal
    }
    pub(crate) fn capabilities(&self) -> Option<Vec<Permission>> {
        self.capabilities.clone()
    }
    pub(crate) fn transport(&self) -> &'static str {
        self.transport
    }
    pub(crate) fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }
}
