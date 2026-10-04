//! 同一持久任务内的 Linux 元数据观察与图发布；来源：Rust D42 / EV-06 / SC-06。
use crate::native_process::{LinuxObservationLease, ProcessNativeSession};
use crate::process_evidence_target::ProcessEvidenceTarget;
use crate::process_native_error::native_error;
use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, ProcessEvidenceFailureCode as Code, ProcessObservationMethod};
use std::cell::Cell;
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
        // 持久 limits 尚未知时只取固定数字 bootstrap；不解码完整 input/authority/scope。
        let bootstrap_deadline = started
            .checked_add(Duration::from_millis(300_000))
            .ok_or(BusinessError::InvalidArgument)?;
        let (limits, bootstrap_raw, bootstrap_allocation) = {
            let control = self
                .try_control_store()?
                .ok_or(BusinessError::BudgetExceeded)?;
            control.with_read_deadline(bootstrap_deadline, |control| {
                control.process_job_limits_with_cost(job_id)
            })?
        };
        let deadline = started
            .checked_add(Duration::from_millis(limits.max_duration_ms()))
            .ok_or(BusinessError::InvalidArgument)?;
        // 回调只读原 token 的绝对期限；初始真实 claim 已验证授权，持久授权读取后绑定原 expiry。
        // 不能在 Store 借用字段的 callback 中重入同一个 Control mutex。
        let expires = Cell::new(None::<u64>);
        let authority_check = || {
            let now =
                crate::job_authorization::unix_seconds().map_err(|_| Code::PermissionDenied)?;
            if expires.get().is_some_and(|expiry| now >= expiry) {
                Err(Code::PermissionDenied)
            } else {
                Ok(())
            }
        };
        let session = ProcessNativeSession::new(&limits, started, cancel, &authority_check)
            .map_err(native_error)?;
        session
            .admit(bootstrap_raw, 1, bootstrap_allocation)
            .map_err(native_error)?;
        let check = || {
            crate::process_execution_fence::checked(
                self,
                job_id,
                owner,
                fence,
                deadline,
                &session,
                |_lease| Ok(()),
            )
        };
        check()?;
        let prepared = (|| {
            let control = self
                .try_control_store()?
                .ok_or(BusinessError::BudgetExceeded)?;
            control
                .with_read_deadline(deadline, |control| -> diskgraph_store::Result<_> {
                    let mut admission = |raw, entries, allocation| {
                        crate::process_execution_target::admit(&session, raw, entries, allocation)
                    };
                    let input = control
                        .process_evidence_job_input_with_admission(job_id, &mut admission)?;
                    let server = control.existing_server_id_with_admission(&mut admission)?;
                    let authority =
                        control.job_request_authority_with_admission(job_id, &mut admission)?;
                    let scope = control.scope_with_admission(input.scope_id(), &mut admission)?;
                    Ok((input, server, authority, scope))
                })
                .map_err(EngineError::from)
        })();
        check()?;
        session.check().map_err(native_error)?;
        let (input, server, authority, scope) = prepared?;
        if input.server_id() != &server {
            return Err(BusinessError::PermissionDenied.into());
        }
        let authority = authority.ok_or(BusinessError::PermissionDenied)?;
        authority.validate_at(crate::job_authorization::unix_seconds()?)?;
        expires.set(authority.expires_at_unix_seconds());
        session.check().map_err(native_error)?;
        if input.limits() != &limits {
            return Err(BusinessError::Conflict.into());
        }
        if input.method() != ProcessObservationMethod::LinuxProcfsV1 {
            return Err(BusinessError::Unsupported.into());
        }
        let target = ProcessEvidenceTarget::for_execution(
            self,
            &input,
            deadline,
            Arc::clone(cancel),
            &session,
            &check,
        )?;
        // base64 解码与 PathBuf 拥有也在同一分配账本内；不以显示路径作为访问目标。
        session
            .admit(
                0,
                0,
                scope.root.raw_b64.len() as u64 + target.locator.raw.len() as u64,
            )
            .map_err(native_error)?;
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
        #[cfg(test)]
        crate::process_execution_tests::before_capture(job_id);
        let opened = LinuxObservationLease::open(&root, relative, &target.epoch, &session);
        check()?;
        let lease = opened.map_err(native_error)?;
        let observed = lease.observe();
        // keeper 的取消可能来自失权或丢失 owner；保持持久真实错误优先。
        check()?;
        let summary = observed.map_err(native_error)?;
        session
            .admit(
                0,
                0,
                (job_id.len() as u64).saturating_mul(2).saturating_add(128),
            )
            .map_err(native_error)?;
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
        lease.verify().map_err(native_error)?;
        session.check().map_err(native_error)?;
        // 同扫描/Git 的 graph→control 顺序。原事务末检不能重入控制库或执行任何 native I/O。
        let mut graph = self.graph()?;
        if !self.accepts_new_work() {
            return Err(BusinessError::ResourceExhausted.into());
        }
        let published_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| BusinessError::InternalError)?
            .as_millis()
            .try_into()
            .map_err(|_| BusinessError::InternalError)?;
        crate::process_execution_fence::checked(
            self,
            job_id,
            owner,
            fence,
            deadline,
            &session,
            |lease_expires| {
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
                            lease_expires,
                            started,
                            input.limits().max_duration_ms(),
                        )
                    },
                )
            },
        )?;
        Ok(())
    }
}
