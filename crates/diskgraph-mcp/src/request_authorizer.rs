use diskgraph_core::{
    Authorizer, Decision, DenyReason, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};

/// 请求级授权器：token 能力与实时数据库策略取交集；本地调用仅检查策略。
pub(crate) struct RequestAuthorizer {
    pub policy: PolicyAuthorizer,
    pub expires_at: Option<u64>,
    pub capabilities: Option<Vec<Permission>>,
}

impl Authorizer for RequestAuthorizer {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_secs())
            .unwrap_or(u64::MAX);
        if self.expires_at.is_some_and(|expires| now >= expires) {
            return Decision::Denied(DenyReason::NoMatchingGrant);
        }
        if self
            .capabilities
            .as_ref()
            .is_some_and(|permissions| !permissions.contains(permission))
        {
            return Decision::Denied(DenyReason::NoMatchingGrant);
        }
        self.policy.decide(principal, permission, scope)
    }

    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }

    fn expires_at_unix_seconds(&self) -> Option<u64> {
        self.expires_at
    }
}
