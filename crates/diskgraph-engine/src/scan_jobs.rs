//! 共享 Engine 的 scan_jobs 职责；原调用与持锁顺序保持。

use crate::job_cancellation_guard::JobCancellationGuard;
use crate::scan_progress_guard;
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, JobRequestAuthority, Permission, PrincipalId, ScopeId,
};
use diskgraph_store::{JobKind, JobRecord, JobState, StoreError};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

impl Engine {
    /// 授权并创建或合并持久索引任务。
    /// 参数：scope_id 为范围；principal/authorizer 为请求身份。
    /// 返回：持久任务或撤权/容量/配额失败。
    /// Creates (or merges into) a durable index job (C02, AI-03).
    pub fn index_scope(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        let authority = JobRequestAuthority::trusted_local(principal.clone(), "trusted-engine")?;
        self.create_scan_job(scope_id, JobKind::Index, &authority, authorizer)
    }
}

impl Engine {
    /// 授权并创建或合并受控重扫任务。
    /// 参数：scope_id 为范围；principal/authorizer 为请求身份。
    /// 返回：持久同步任务或门禁失败。
    /// Explicit controlled rescan of a registered scope (C03): merges into any
    /// active job for the scope and publishes a fresh snapshot + revision.
    pub fn sync_scope(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        let authority = JobRequestAuthority::trusted_local(principal.clone(), "trusted-engine")?;
        self.create_scan_job(scope_id, JobKind::Sync, &authority, authorizer)
    }
}

impl Engine {
    /// 以不可变请求身份执行原有容量、配额与授权入队门禁。
    /// 参数：scope_id/kind 固定工作类型，authority 固定主体与上限，authorizer 为当前请求授权。
    /// 返回：持久记录或门禁失败；与 Store 的原子入队检查共同生效。
    pub(super) fn create_scan_job(
        &self,
        scope_id: &ScopeId,
        kind: JobKind,
        authority: &JobRequestAuthority,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        let principal = authority.principal();
        if !authority.allows(
            &Permission::IndexWrite,
            crate::job_authorization::unix_seconds()?,
        ) {
            return Err(BusinessError::PermissionDenied.into());
        }
        {
            let control = self.control()?;
            // 保留已获授权调用方对已撤 scope 的既有 conflict/退出码契约。
            // 未获 token 或实时数据库 grant 的主体仍只能得到 permission_denied。
            if control.scope(scope_id)?.revoked
                && matches!(
                    authorizer.decide(principal, &Permission::IndexWrite, scope_id),
                    diskgraph_core::Decision::Allowed
                )
                && (control.policy_state()?.is_none()
                    || matches!(
                        control
                            .authorizer()?
                            .decide(principal, &Permission::IndexWrite, scope_id),
                        diskgraph_core::Decision::Allowed
                    ))
            {
                return Err(EngineError::Store(StoreError::Conflict(format!(
                    "scope {scope_id} is revoked"
                ))));
            }
        }
        self.require(authorizer, principal, &Permission::IndexWrite, scope_id)?;
        if !self.accepts_new_work() {
            return Err(EngineError::Business(BusinessError::ResourceExhausted));
        }
        let mut control = self.control()?;
        if control.scope(scope_id)?.revoked {
            return Err(EngineError::Store(StoreError::Conflict(format!(
                "scope {scope_id} is revoked"
            ))));
        }
        let job = control
            .create_job_with_authority(
                scope_id,
                kind,
                authority,
                u64::from(self.max_active_jobs_per_principal),
            )?
            .ok_or(EngineError::Business(BusinessError::ResourceExhausted))?;
        drop(control);
        Ok(job)
    }
}

impl Engine {
    /// 按任务实际范围授权取消并通知本机取消标志。
    /// 参数：job_id 为任务；principal/authorizer 为请求身份。
    /// 返回：取消意图持久化成功或授权/查询失败。
    /// Cancels a job (C26 semantics arrive in P5; P1 cancels scans).
    /// Queued jobs are cancelled immediately; running jobs observe the flag
    /// between walk batches and never advance `latest`.
    pub fn cancel_job(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        let prior_cancel = self.cancellations()?.get(job_id).cloned();
        let job = self.control()?.job(job_id)?;
        self.require(
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        let terminal = {
            let mut control = self.control()?;
            control.request_cancel(job_id)?;
            matches!(
                control.job(job_id)?.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        };
        if let Some(flag) = self.cancellations()?.get(job_id) {
            flag.store(true, Ordering::SeqCst);
        }
        // DB 已终结才释放进入调用时的标志；Running 仍由活动 owner 的 guard 负责。
        let _cleanup =
            if terminal { prior_cancel } else { None }.map(|flag| JobCancellationGuard {
                entries: &self.cancellations,
                job_id: job_id.to_owned(),
                flag,
            });
        Ok(())
    }
}

impl Engine {
    /// 条件认领并在 fencing/续租下执行一次持久扫描。
    /// 参数：job_id 为指定任务，owner 为执行 owner。
    /// 返回：终态记录或认领/扫描/发布失败。
    /// Claims and runs one job to a terminal state, returning the durable
    /// record (RT-01: reconnection queries this instead of the connection).
    pub fn run_job(&self, job_id: &str, owner: &str) -> Result<JobRecord, EngineError> {
        self.run_job_with_authority_mode(job_id, owner, false)
    }

    /// 复用同一执行链，按宿主信任边界选择是否允许缺来源的历史任务。
    /// 参数：job_id/owner 绑定代次，require_authority 为远程严格模式。
    /// 返回：真实终态或执行失败；已有请求身份在两种模式下均须复验。
    pub(super) fn run_job_with_authority_mode(
        &self,
        job_id: &str,
        owner: &str,
        require_authority: bool,
    ) -> Result<JobRecord, EngineError> {
        let prior_cancel = self.cancellations()?.get(job_id).cloned();
        if let Some(recovered) = self
            .recover_git_publication(job_id, owner)?
            .or(self.recover_process_publication(job_id, owner)?)
        {
            // 对账成功才清理进入本次调用时的旧标志，不触碰后续并发代次。
            let _cleanup = prior_cancel.map(|flag| JobCancellationGuard {
                entries: &self.cancellations,
                job_id: job_id.to_owned(),
                flag,
            });
            return Ok(recovered);
        }
        let claimed = {
            let mut control = self.control()?;
            if require_authority {
                control.claim_job_once_strict(job_id, owner)
            } else {
                control.claim_job_once(job_id, owner)
            }
        };
        let claimed = match claimed {
            Ok(claimed) => claimed,
            Err(error) => {
                // 认领可能已把排队到期/拒权写为终态；清理失败不能替换原始业务错误。
                // 先释放控制锁再操作取消表，不引入 control/cancellation 反向嵌套。
                let terminal = self
                    .control()
                    .and_then(|control| {
                        Ok(matches!(
                            control.job(job_id)?.state,
                            JobState::Completed | JobState::Failed | JobState::Cancelled
                        ))
                    })
                    .unwrap_or(false);
                let _cleanup =
                    if terminal { prior_cancel } else { None }.map(|flag| JobCancellationGuard {
                        entries: &self.cancellations,
                        job_id: job_id.to_owned(),
                        flag,
                    });
                return Err(error.into());
            }
        };
        // 认领后的图锁等待、旧暂存清理与权限准备也消耗同一运行预算。
        let scan_started = std::time::Instant::now();
        let _progress_cleanup = scan_progress_guard::ScanProgressGuard {
            entries: &self.scan_progress,
            key: (job_id.to_owned(), claimed.fencing_token),
        };
        // An expired owner cannot write after the new claim. Reclaim only
        // generations strictly older than the active fencing token.
        self.graph()?
            .clear_stale_job_staging(job_id, claimed.fencing_token)?;
        // 每个认领代次独立取消标志；过期 owner 的标志不能取消新 owner。
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancellations()?
            .insert(job_id.to_owned(), Arc::clone(&cancel));
        let _cancellation_cleanup = JobCancellationGuard {
            entries: &self.cancellations,
            job_id: job_id.to_owned(),
            flag: Arc::clone(&cancel),
        };
        // 租约覆盖转换、staging 和 collector 阶段，不能只在扫描进度循环续租。
        let outcome = std::thread::scope(|threads| {
            let (stop, receiver) = std::sync::mpsc::channel();
            let cancel_ref = &cancel;
            let fence = claimed.fencing_token;
            let keeper = threads.spawn(move || {
                let mut heartbeat = std::time::Instant::now();
                // 转换与项目采集也属于执行窗口；同一 keeper 每 20ms 复验权限，租约仍每 5s 续租。
                // 执行栈 unwind 会释放 stop sender；断连是停止，不得忙循环续租阻止 scope join。
                while matches!(
                    receiver.recv_timeout(std::time::Duration::from_millis(20)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    let checked = self.control().and_then(|mut store| {
                        store.with_job_fence(job_id, owner, fence, || Ok(()))?;
                        if heartbeat.elapsed() >= std::time::Duration::from_secs(5) {
                            store.heartbeat_fenced(job_id, owner, fence)?;
                            heartbeat = std::time::Instant::now();
                        }
                        Ok(())
                    });
                    if checked.is_err() {
                        cancel_ref.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            });
            let result = match claimed.kind {
                JobKind::GitEvidence => self.execute_git_evidence(
                    job_id,
                    owner,
                    claimed.fencing_token,
                    &cancel,
                    scan_started,
                ),
                JobKind::ProcessEvidence => self.admit_process_execution(
                    job_id,
                    owner,
                    claimed.fencing_token,
                    &cancel,
                    scan_started,
                ),
                JobKind::Index | JobKind::Sync => {
                    self.execute_scan(job_id, owner, claimed.fencing_token, &cancel, scan_started)
                }
            };
            let _ = stop.send(());
            let _ = keeper.join();
            result
        });
        #[cfg(test)]
        if claimed.kind == JobKind::GitEvidence && outcome.is_ok() {
            crate::git_evidence_execution_tests::after_publication(job_id)?;
        }
        #[cfg(all(test, target_os = "linux"))]
        let outcome = if claimed.kind == JobKind::ProcessEvidence && outcome.is_ok() {
            crate::process_execution_tests::after_commit(job_id)
        } else {
            outcome
        };
        // 图事务可能已成功而 control 的独立提交失败。不能把已提交事实改为 Failed，
        // 保留 Running 供租约到期后按唯一回执对账，且绝不在这里重新采集。
        if outcome.is_err() {
            let committed = match claimed.kind {
                JobKind::GitEvidence => self.graph()?.job_publication_receipt(job_id)?.is_some(),
                JobKind::ProcessEvidence => self
                    .graph()?
                    .process_job_publication_receipt(job_id)?
                    .is_some(),
                JobKind::Index | JobKind::Sync => false,
            };
            if committed {
                return outcome.map(|()| claimed);
            }
        }
        let final_state = if outcome.is_ok() {
            JobState::Completed
        } else if self
            .control()?
            .cancellation_requested(job_id, claimed.fencing_token)?
        {
            JobState::Cancelled
        } else {
            JobState::Failed
        };
        if outcome.is_err() {
            let staging_id = format!("{job_id}:{}", claimed.fencing_token);
            self.graph()?.clear_staging(&staging_id)?;
        }
        let record = if claimed.kind == JobKind::GitEvidence {
            let failure = outcome
                .as_ref()
                .err()
                .map(|error| crate::git_evidence_failure::execution(error, final_state));
            self.control()?.finish_git_job_fenced(
                job_id,
                owner,
                claimed.fencing_token,
                final_state,
                failure.as_ref(),
            )?
        } else if claimed.kind == JobKind::ProcessEvidence {
            let method = self.control()?.process_evidence_job_input(job_id)?.method();
            let failure = outcome.as_ref().err().map(|error| {
                crate::process_evidence_admission::failure(error, final_state, method)
            });
            self.control()?.finish_process_job_fenced(
                job_id,
                owner,
                claimed.fencing_token,
                final_state,
                failure.as_ref(),
            )?
        } else {
            self.control()?
                .finish_job_fenced(job_id, owner, claimed.fencing_token, final_state)?
        };
        self.graph()?
            .clear_stale_job_staging(job_id, claimed.fencing_token)?;
        outcome?;
        Ok(record)
    }
}

impl Engine {
    /// 按任务范围授权读取当前 fence 的实际进度。
    /// 参数：job_id、principal、authorizer 绑定任务与请求身份。
    /// 返回：状态及可选真实计数；未观察阶段不编造进度。
    /// 按作业实际 scope 授权读取真实扫描进度；非扫描阶段不捏造计数。
    pub fn job_progress(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<serde_json::Value, EngineError> {
        let job = self.job_status(job_id)?;
        self.require(
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        let entries = self
            .scan_progress
            .lock()
            .map_err(|_| EngineError::Business(BusinessError::InternalError))?;
        let counts=entries.get(&(job_id.to_owned(),job.fencing_token)).map(|p|serde_json::json!({"files":p.files,"directories":p.dirs,"bytes":p.bytes.to_string(),"read_errors":p.errors}));
        Ok(serde_json::json!({"job_id":job_id,"state":job.state,"observed":counts}))
    }
}

impl Engine {
    /// 解析完成任务自身的 job/fence revision 并授权。
    /// 参数：job_id、principal、authorizer 为任务与请求身份。
    /// 返回：该代次 revision 或未完成/授权失败。
    /// 解析已完成作业实际发布的 revision；认领代次固定，不能返回后来更新的 latest。
    pub fn revision_for_job(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<String, EngineError> {
        let job = self.job_status(job_id)?;
        self.require(
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        if job.state != JobState::Completed {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        let revision = if job.kind == JobKind::GitEvidence {
            self.graph()?
                .job_publication_receipt(job_id)?
                .ok_or(BusinessError::Conflict)?
                .revision_id()
                .to_owned()
        } else if job.kind == JobKind::ProcessEvidence {
            self.graph()?
                .process_job_publication_receipt(job_id)?
                .ok_or(BusinessError::Conflict)?
                .revision_id()
                .to_owned()
        } else {
            format!("rev-{}-{}", job_id, job.fencing_token)
        };
        self.authorize_revision(Some(&job.scope_id), &revision, principal, authorizer)?;
        Ok(revision)
    }
}

impl Engine {
    /// 可信内部读取持久任务，不独立授权请求。
    /// 参数：job_id 为任务标识。
    /// 返回：持久任务记录或查询失败。
    /// Loads one durable job record (reconnect-safe business state, MCP-05 seed).
    pub fn job_status(&self, job_id: &str) -> Result<JobRecord, EngineError> {
        Ok(self.control()?.job(job_id)?)
    }
}

impl Engine {
    /// 可信执行器终结指定已过期且撤销/取消的任务。
    /// 参数：job_id 为指定任务；远程适配器须先授权。
    /// 返回：最新持久任务；不抢占存活租约或其他任务。
    /// 可信执行器按任务 ID 回收过期且取消/撤销的 owner；返回持久任务状态。
    /// 不认领其他任务、不抢占存活租约，远程调用仍需先做任务范围授权。
    pub fn settle_expired_job(&self, job_id: &str) -> Result<JobRecord, EngineError> {
        let (changed, job) = {
            let mut control = self.control()?;
            let changed = control.reap_unclaimable_job(job_id)?;
            (changed, control.job(job_id)?)
        };
        if changed {
            // 此代次已持久终结，旧 owner 的 fence 无法再写/发布；只删除图暂存数据。
            self.graph()?
                .clear_stale_job_staging(job_id, job.fencing_token.saturating_add(1))?;
        }
        Ok(job)
    }
}

impl Engine {
    /// 可信 runner 清理不可认领任务并读取活动队列。
    /// 参数：无。
    /// 返回：排队任务列表或控制库失败。
    /// Every queued job across scopes; the job runner drains this list.
    pub fn queued_jobs(&self) -> Result<Vec<JobRecord>, EngineError> {
        let mut control = self.control()?;
        control.reap_unclaimable_jobs()?;
        Ok(control.list_queued_jobs()?)
    }
}

impl Engine {
    /// 为后台 runner 读取固定上限的候选页，保留原 scope/取消回收语义。
    /// 参数：maximum 为本次最多可物化的任务记录数。
    /// 返回：按持久创建时间及 ID 排序的候选；请求授权在逐项认领时原子复验。
    pub(super) fn queued_jobs_limited(
        &self,
        maximum: usize,
    ) -> Result<Vec<JobRecord>, EngineError> {
        let mut control = self.control()?;
        control.reap_unclaimable_jobs()?;
        Ok(control.list_queued_jobs_limited(maximum)?)
    }
}
