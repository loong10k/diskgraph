//! Process 持久请求入口；来源：Rust D42，身份只来自扫描旁表与真实请求 authority。
use crate::process_evidence_target::ProcessEvidenceTarget;
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, JobRequestAuthority, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessObservationMethod, ScopeId,
};
use diskgraph_store::JobRecord;
use std::time::{Duration, Instant};

impl Engine {
    /// 参数：真实scope、固定revision/node、已验证请求身份和当前授权；返回：持久Process任务或明确资格拒绝。
    /// 仅 MetadataRead/IndexWrite 原token∩liveDB，无 ContentRead；不接受路径/程序/预算等客户端字段。
    /// 旧无 epoch 与当前未资格平台 Unsupported 且零入队，源访问留给同owner执行阶段。
    pub fn process_evidence_scope_with_authority(
        &self,
        scope: &ScopeId,
        base_revision: &str,
        node_id: u64,
        authority: &JobRequestAuthority,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        let deadline = Instant::now() + Duration::from_secs(1);
        if base_revision.is_empty() || base_revision.len() > 128 || node_id == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let check = || self.require_process_entry(scope, authority, authorizer, deadline);
        check()?;
        let target = ProcessEvidenceTarget::load(
            self,
            scope,
            base_revision,
            node_id,
            deadline,
            None,
            &check,
        )?;
        let (registered, server) = {
            let control = self
                .try_control_store()?
                .ok_or(BusinessError::BudgetExceeded)?;
            control.with_read_deadline(deadline, |control| {
                let actual = control.scope(scope)?;
                if actual.revoked {
                    return Err(EngineError::Business(BusinessError::PermissionDenied));
                }
                let registered = actual
                    .root
                    .to_native_path()
                    .map_err(|_| BusinessError::Unsupported)?;
                Ok::<_, EngineError>((registered, control.existing_server_id()?))
            })?
        };
        let native = target
            .locator
            .to_native_path()
            .map_err(|_| BusinessError::Unsupported)?;
        let relative = native
            .strip_prefix(&registered)
            .map_err(|_| BusinessError::PermissionDenied)?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || target.snapshot_id.is_empty()
            || target.snapshot_id.len() > 128
        {
            return Err(BusinessError::Unsupported.into());
        }
        let input = ProcessEvidenceJobInput::new(
            server,
            scope.clone(),
            base_revision.to_owned(),
            node_id,
            ProcessObservationMethod::LinuxProcfsV1,
            target.epoch,
            ProcessEvidenceLimits::default(),
        )
        .map_err(|_| BusinessError::InvalidArgument)?;
        if !self.accepts_new_work() {
            return Err(BusinessError::ResourceExhausted.into());
        }
        check()?;
        let mut control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        // 创建事务自己再次验证原权限/expiry，不能把只读阶段的权限当成长期委派。
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let job = control
            .create_process_evidence_job_until(
                &input,
                authority,
                u64::from(self.max_active_jobs_per_principal),
                deadline,
            )?
            .ok_or(BusinessError::ResourceExhausted)?;
        Ok(job)
    }
    /// 参数：真实scope、原请求与当前授权及原期限；返回：所有原能力和实时grant仍有效。
    pub(super) fn require_process_entry(
        &self,
        scope: &ScopeId,
        authority: &JobRequestAuthority,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<(), EngineError> {
        let control = self
            .try_control_store()?
            .ok_or(BusinessError::BudgetExceeded)?;
        control.with_read_deadline(deadline, |control| {
            let now = crate::job_authorization::unix_seconds()?;
            if control.scope(scope)?.revoked {
                return Err(BusinessError::PermissionDenied.into());
            }
            for permission in diskgraph_store::JobKind::ProcessEvidence.required_permissions() {
                if !authority.allows(permission, now) {
                    return Err(BusinessError::PermissionDenied.into());
                }
                Self::require_with_control(
                    control,
                    authorizer,
                    authority.principal(),
                    permission,
                    scope,
                )?;
            }
            for permission in diskgraph_store::JobKind::ProcessEvidence.required_permissions() {
                if control.live_permission(authority.principal(), permission, scope)? == Some(false)
                {
                    return Err(BusinessError::PermissionDenied.into());
                }
            }
            authority.validate_at(crate::job_authorization::unix_seconds()?)?;
            if Instant::now() >= deadline {
                return Err(BusinessError::BudgetExceeded.into());
            }
            Ok(())
        })
    }
}
