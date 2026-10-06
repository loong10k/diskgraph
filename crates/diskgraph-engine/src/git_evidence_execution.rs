//! 原任务 owner/期限内的 Git 采样与图事务发布；来源：原生 Rust EC-02 / EC-04 / SC-06。

use crate::git_evidence_target::GitEvidenceTarget;
use crate::live_evidence::EvidenceProbeSession;
use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

impl Engine {
    /// 参数：job/owner/fence 为原认领代次，cancel 为 keeper 共用标志，started 为成功认领后唯一时钟。
    /// 返回：完整安全摘要及 receipt 同图事务发布成功，或真实授权/预算/采样/发布失败。
    pub(super) fn execute_git_evidence(
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
            let input = control.git_evidence_job_input(job_id)?;
            let authority = control
                .job_request_authority(job_id)?
                .ok_or(BusinessError::PermissionDenied)?;
            let scope = control.scope(input.scope_id())?;
            (input, authority, scope)
        };
        if input.server_id() != &self.server_id()? {
            return Err(BusinessError::PermissionDenied.into());
        }
        let deadline = started
            .checked_add(Duration::from_millis(input.limits().max_duration_ms()))
            .ok_or(BusinessError::InvalidArgument)?;
        let root = scope
            .root
            .to_native_path()
            .map_err(|_| BusinessError::Unsupported)?;
        let target = GitEvidenceTarget::load(self, &input, deadline, Some(Arc::clone(cancel)))?;
        #[cfg(not(windows))]
        let mut session =
            EvidenceProbeSession::for_git_job(input.limits(), started, Arc::clone(cancel))
                .map_err(|error| EngineError::Business(error.business()))?;
        #[cfg(windows)]
        let mut session = match self.probe_host.as_ref() {
            Some(host) => EvidenceProbeSession::for_git_job_with_probe_host(
                input.limits(),
                started,
                Arc::clone(cancel),
                host,
            ),
            None => EvidenceProbeSession::for_git_job(input.limits(), started, Arc::clone(cancel)),
        }
        .map_err(|error| EngineError::Business(error.business()))?;
        #[cfg(test)]
        crate::git_evidence_execution_tests::before_capture(job_id);
        let observed =
            session.sample_git_indexed(Path::new("git"), &root, &target.locator, &target.identity);
        // keeper 的取消可能源于失权/owner 丢失；先执行真实 fence，不能把它误报为工具 unsupported。
        self.control()?
            .with_job_fence(job_id, owner, fence, || Ok(()))?;
        let sample = observed.map_err(|error| EngineError::Business(error.business()))?;
        let run = format!("git-{job_id}-{fence}");
        let revision = format!("rev-{job_id}-{fence}");
        let batch = crate::git_evidence_batch::build(&input, &target.snapshot_id, &run, &sample)?;
        #[cfg(test)]
        crate::git_evidence_execution_tests::before_publication(job_id);
        // 与扫描相同的 graph→control 锁序；撤权事务与图提交前检查不能倒置。
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
            graph.publish_git_collector_revision_checked(
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

    /// 参数：job_id/owner 为拟执行任务；返回：已提交事实的幂等控制终态，或尚无回执。
    /// 先对账图回执再进行严格认领，原请求过期不能否认已经提交的事实；不因此重新采样。
    pub(super) fn recover_git_publication(
        &self,
        job_id: &str,
        owner: &str,
    ) -> Result<Option<diskgraph_store::JobRecord>, EngineError> {
        let job = self.control()?.job(job_id)?;
        if job.kind != diskgraph_store::JobKind::GitEvidence {
            return Ok(None);
        }
        let receipt = self.graph()?.job_publication_receipt(job_id)?;
        match receipt {
            Some(receipt) => Ok(Some(
                self.control()?
                    .recover_committed_git_job(job_id, owner, &receipt)?,
            )),
            None => Ok(None),
        }
    }
}
