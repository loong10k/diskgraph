//! 持久任务唯一执行链；来源：原生 Rust Engine scan_jobs 职责提取。

use crate::job_cancellation_guard::JobCancellationGuard;
use crate::job_execution_stop_reason::JobExecutionStopReason;
use crate::job_request_cancel_bridge::JobRequestCancelBridge;
use crate::scan_progress_guard;
use crate::{Engine, EngineError};
use diskgraph_store::{JobKind, JobRecord, JobState};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

impl Engine {
    /// 复用同一执行链，按宿主信任边界选择是否允许缺来源的历史任务。
    /// 参数：job_id/owner 绑定代次，require_authority 为远程严格模式。
    /// 返回：真实终态或执行失败；已有请求身份在两种模式下均须复验。
    pub(super) fn execute_job_with_stop_signals(
        &self,
        job_id: &str,
        owner: &str,
        require_authority: bool,
        signals: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>,
    ) -> Result<JobRecord, EngineError> {
        let prior_cancel = self.cancellations()?.get(job_id).cloned();
        let recovered = if let Some(scan) = self.recover_scan_publication(job_id, owner)? {
            Some(scan)
        } else if let Some(git) = self.recover_git_publication(job_id, owner)? {
            Some(git)
        } else {
            self.recover_process_publication(job_id, owner)?
        };
        if let Some(recovered) = recovered {
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
        // 每个认领代次独立取消标志；不复用 caller flag，也不触碰后来 owner 的 Arc。
        let cancel = Arc::new(AtomicBool::new(false));
        let stop_reason = JobExecutionStopReason::new();
        let bridge = JobRequestCancelBridge::new(
            self,
            &claimed,
            signals
                .as_ref()
                .map(|(request, denied)| (request.as_ref(), denied.as_ref())),
            cancel.as_ref(),
        );
        let preparation = bridge.check();
        let preparation = if preparation.is_ok() {
            // 原图锁等待和旧暂存清理沿用认领后的时钟；等待后再次消费原 caller 信号。
            self.graph()?
                .clear_stale_job_staging(job_id, claimed.fencing_token)?;
            bridge.check()
        } else {
            preparation
        };
        let _cancellation_cleanup = if preparation.is_ok() {
            #[cfg(test)]
            crate::job_stop_registration_tests::before_registration(job_id);
            self.cancellations()?
                .insert(job_id.to_owned(), Arc::clone(&cancel));
            Some(JobCancellationGuard {
                entries: &self.cancellations,
                job_id: job_id.to_owned(),
                flag: Arc::clone(&cancel),
            })
        } else {
            None
        };
        // Map Mutex 可能在前次检查之后等待；登记完成后再次消费原请求信号，
        // 不能让该等待窗口内的请求跨进真实 dispatch。旧无信号入口不增加 DB 读取。
        let preparation = preparation.and_then(|()| bridge.check());
        // 租约覆盖转换、staging 和 collector 阶段，不能只在扫描进度循环续租。
        let outcome = preparation.and_then(|()| {
            #[cfg(test)]
            crate::job_stop_registration_tests::before_dispatch(job_id);
            std::thread::scope(|threads| {
                let (stop, receiver) = std::sync::mpsc::channel();
                let cancel_ref = &cancel;
                let bridge_ref = &bridge;
                let stop_reason_ref = &stop_reason;
                let fence = claimed.fencing_token;
                #[cfg(test)]
                let mut observations = crate::job_stop_cause_hooks::take_keeper(&claimed);
                let keeper = threads.spawn(move || {
                    let mut heartbeat = std::time::Instant::now();
                    // 转换与项目采集也属于执行窗口；同一 keeper 每 20ms 复验权限，租约仍每 5s 续租。
                    // 执行栈 unwind 会释放 stop sender；断连是停止，不得忙循环续租阻止 scope join。
                    while matches!(
                        receiver.recv_timeout(std::time::Duration::from_millis(20)),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        #[cfg(test)]
                        observations.before_check();
                        let checked = self.control().and_then(|mut store| {
                            store.with_job_fence(job_id, owner, fence, || Ok(()))?;
                            bridge_ref.check_with_control(&mut store)?;
                            if heartbeat.elapsed() >= std::time::Duration::from_secs(5) {
                                store.heartbeat_fenced(job_id, owner, fence)?;
                                heartbeat = std::time::Instant::now();
                            }
                            Ok(())
                        });
                        if let Err(error) = checked {
                            // 原因先取得所有权，再发布停止位；消费者不必根据 Atomic 猜测拒权/SQL 错误。
                            stop_reason_ref.record(error);
                            cancel_ref.store(true, Ordering::SeqCst);
                            #[cfg(test)]
                            stop_reason_ref.observe(|error| observations.after_error(error));
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
                    JobKind::Index | JobKind::Sync => self.execute_scan(
                        job_id,
                        owner,
                        claimed.fencing_token,
                        &cancel,
                        scan_started,
                        &stop_reason,
                    ),
                };
                #[cfg(test)]
                crate::job_stop_cause_hooks::after_execution(job_id, &result);
                let _ = stop.send(());
                if let Err(panic) = keeper.join() {
                    // 已经实际 join；保留原 panic payload，不猜测为 Poisoned 或协作取消。
                    std::panic::resume_unwind(panic);
                }
                result
            })
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
                JobKind::Index | JobKind::Sync => {
                    self.graph()?.scan_publication_receipt(job_id)?.is_some()
                }
            };
            if committed {
                return outcome.map(|()| claimed);
            }
        }
        // receipt 已提交时前面保留原事实；这里只处理尚未提交的本代协作停止。
        let outcome = stop_reason.restore_commit_stop(outcome);
        let outcome = bridge.preserve_outcome(outcome);
        let final_state = if outcome.is_ok() {
            JobState::Completed
        } else if bridge.denial_observed() {
            JobState::Failed
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
        // 显式 WAL 维护可能远超 30 秒租约；必须在 Completed 已持久化且控制锁释放后执行。
        // 维护失败不能把已提交事实改为 Failed，也不能让旧 owner 因维护期间租约到期重扫。
        if matches!(claimed.kind, JobKind::Index | JobKind::Sync)
            && let Ok(graph) = self.graph()
        {
            let _ = graph.checkpoint_after_publication();
        }
        Ok(record)
    }
}
