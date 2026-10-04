//! 同一持久任务内的 Linux 元数据观察与图发布；来源：Rust D42 / EV-06 / SC-06。
use crate::native_process::ProcessNativeSession;
use crate::process_evidence_target::ProcessEvidenceTarget;
use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, ProcessEvidenceFailureCode as Code, ProcessObservationMethod};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

impl Engine {
    /// 参数：原 job/owner/fence、keeper 共用取消与认领唯一时钟；返回：真实采样和原子回执发布结果。
    /// 不读取文件正文，不把方法可见域 Partial 解释成全系统无占用；Mac/Windows 不借此启用。
    pub(super) fn execute_process_evidence(
        &self,
        job_id: &str,
        owner: &str,
        fence: u64,
        cancel: &Arc<AtomicBool>,
        started: Instant,
    ) -> Result<(), EngineError> {
        let (input, authority, scope) = {
            let mut control = self.control()?;
            control.with_job_fence(job_id, owner, fence, || Ok(()))?;
            let input = control.process_evidence_job_input(job_id)?;
            if input.server_id() != &control.existing_server_id()? {
                return Err(BusinessError::PermissionDenied.into());
            }
            let authority = control
                .job_request_authority(job_id)?
                .ok_or(BusinessError::PermissionDenied)?;
            let scope = control.scope(input.scope_id())?;
            (input, authority, scope)
        };
        if input.method() != ProcessObservationMethod::LinuxProcfsV1 {
            return Err(BusinessError::Unsupported.into());
        }
        let deadline = started
            .checked_add(Duration::from_millis(input.limits().max_duration_ms()))
            .ok_or(BusinessError::InvalidArgument)?;
        let check = || {
            self.control()?
                .with_job_fence(job_id, owner, fence, || Ok(()))?;
            Ok(())
        };
        let target = ProcessEvidenceTarget::load(
            self,
            input.scope_id(),
            input.base_revision_id(),
            input.node_id(),
            deadline,
            Some(Arc::clone(cancel)),
            &check,
        )?;
        if &target.epoch != input.indexed_epoch() {
            return Err(BusinessError::Conflict.into());
        }
        let root = scope
            .root
            .to_native_path()
            .map_err(|_| BusinessError::Unsupported)?;
        let native = target
            .locator
            .to_native_path()
            .map_err(|_| BusinessError::Unsupported)?;
        let relative = native
            .strip_prefix(&root)
            .map_err(|_| BusinessError::PermissionDenied)?;
        // 回调只复验原 token 的绝对期限；实时权限/owner 由原 20ms keeper 和各阶段真实 fence 负责。
        let authority_check = || {
            let now =
                crate::job_authorization::unix_seconds().map_err(|_| Code::PermissionDenied)?;
            authority
                .validate_at(now)
                .map_err(|_| Code::PermissionDenied)
        };
        let session = ProcessNativeSession::new(input.limits(), started, cancel, &authority_check)
            .map_err(native_error)?;
        #[cfg(test)]
        crate::process_execution_tests::before_capture(job_id);
        let observed = session.observe_linux_file(&root, relative, input.indexed_epoch());
        // keeper 的取消可能来自失权或丢失 owner；保持持久真实错误优先，不猜测工具输出。
        check()?;
        let summary = observed.map_err(native_error)?;
        let run = format!("process-{job_id}-{fence}");
        let revision = format!("rev-{job_id}-{fence}");
        let batch = crate::process_evidence_batch::build(
            &input,
            &target.snapshot_id,
            &run,
            &summary,
            &session,
        )?;
        #[cfg(test)]
        crate::process_execution_tests::before_publication(job_id);
        // 原权限/owner 拒绝优先于 keeper 的取消提示和纯预算末检。
        check()?;
        session.check().map_err(native_error)?;
        // 同扫描/Git 的 graph→control 顺序。原事务末检不能重入控制库或执行任何 native I/O。
        let mut graph = self.graph()?;
        let mut control = self.control()?;
        if !self.accepts_new_work() {
            return Err(BusinessError::ResourceExhausted.into());
        }
        control.heartbeat_fenced(job_id, owner, fence)?;
        let lease = control.job(job_id)?.lease_expires_unix_ms;
        let published_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| BusinessError::InternalError)?
            .as_millis()
            .try_into()
            .map_err(|_| BusinessError::InternalError)?;
        control.with_job_fence(job_id, owner, fence, || {
            graph.publish_process_collector_revision_checked(
                job_id,
                &input,
                (fence, published_at),
                &revision,
                &batch,
                || {
                    crate::job_authorization::check_scan_commit(
                        Some(&authority),
                        cancel,
                        lease,
                        started,
                        input.limits().max_duration_ms(),
                    )
                },
            )
        })?;
        Ok(())
    }
}

/// 参数：原生有类型失败；返回：对应业务类别，不从错误字符串猜测身份或授权。
pub(super) fn native_error(code: Code) -> EngineError {
    match code {
        Code::BudgetExceeded => BusinessError::BudgetExceeded,
        Code::Timeout => BusinessError::Timeout,
        Code::Cancelled | Code::Conflict => BusinessError::Conflict,
        Code::PermissionDenied => BusinessError::PermissionDenied,
        Code::Unsupported => BusinessError::Unsupported,
        Code::Unavailable => BusinessError::Unavailable,
        Code::InternalError => BusinessError::InternalError,
    }
    .into()
}
