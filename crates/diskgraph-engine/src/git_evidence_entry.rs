//! 持久 Git 请求入口；来源：原生 Rust C03 / EC-02，权限来自真实适配器上下文。

use crate::git_evidence_target::GitEvidenceTarget;
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, GitEvidenceJobInput, GitEvidenceLimits, JobRequestAuthority, ScopeId,
};
use diskgraph_store::{JobKind, JobRecord};
use std::time::{Duration, Instant};

impl Engine {
    /// 为显式索引目录创建或合并持久 Git 采集任务。
    /// 参数：scope/base_revision/node_id 固定索引目标，authority 是经验证的原请求来源，authorizer 是当前能力。
    /// 返回：持久 job；不接收客户端路径、argv、网络选项或预算，入队不会隐式扫描/采集。
    pub fn git_evidence_scope_with_authority(
        &self,
        scope: &ScopeId,
        base_revision: &str,
        node_id: u64,
        authority: &JobRequestAuthority,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        let now = crate::job_authorization::unix_seconds()?;
        for permission in JobKind::GitEvidence.required_permissions() {
            if !authority.allows(permission, now) {
                return Err(BusinessError::PermissionDenied.into());
            }
            self.require(authorizer, authority.principal(), permission, scope)?;
        }
        if self.scope(scope)?.revoked {
            return Err(BusinessError::PermissionDenied.into());
        }
        let input = GitEvidenceJobInput::new(
            self.server_id()?,
            scope.clone(),
            base_revision.to_owned(),
            node_id,
            GitEvidenceLimits::default(),
        )
        .map_err(|_| BusinessError::InvalidArgument)?;
        let deadline = Instant::now() + Duration::from_millis(1000);
        // 入队只读不可变索引记录；源打开及 indexed 身份匹配属于执行阶段。
        GitEvidenceTarget::load(self, &input, deadline, None)?;
        if !self.accepts_new_work() {
            return Err(BusinessError::ResourceExhausted.into());
        }
        let mut control = self.control()?;
        for permission in JobKind::GitEvidence.required_permissions() {
            Self::require_with_control(
                &control,
                authorizer,
                authority.principal(),
                permission,
                scope,
            )?;
        }
        let job = control
            .create_git_evidence_job(
                &input,
                authority,
                u64::from(self.max_active_jobs_per_principal),
            )?
            .ok_or(BusinessError::ResourceExhausted)?;
        drop(control);
        Ok(job)
    }
}
