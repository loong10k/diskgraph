//! 扫描 worker、持久作业轮询和取消信号协调，保持 Engine 为执行主体。
use crate::JobHandle;
use crate::api_result::ApiResult;
use crate::job_state::JobState;
use crate::native_realm::local_principal;
use crate::native_runner_guard::NativeRunnerGuard;
use diskgraph_core::{BusinessError, PrincipalId};
use diskgraph_engine::{Engine, EngineError};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn authorized_job_progress(engine: &Engine, job_id: &str) -> ApiResult {
    let principal = local_principal()?;
    authorized_progress(engine, job_id, &principal).map_err(progress_error)
}

// 原 FFI 明确拒权使用稳定 wire；不检查任意错误文本，不改变其他错误消息。
fn progress_error(error: EngineError) -> String {
    if matches!(
        error.primary(),
        EngineError::Business(BusinessError::PermissionDenied)
    ) {
        "permission_denied".into()
    } else {
        error.to_string()
    }
}

fn authorized_progress(
    engine: &Engine,
    job_id: &str,
    principal: &PrincipalId,
) -> Result<Value, EngineError> {
    let policy = engine.policy_authorizer()?;
    let progress = engine.job_progress(job_id, principal, &policy)?;
    let job = engine.job_status(job_id)?;
    if engine.control_store()?.live_permission(
        principal,
        &diskgraph_core::Permission::MetadataRead,
        &job.scope_id,
    )? == Some(false)
    {
        return Err(BusinessError::PermissionDenied.into());
    }
    if job.state == diskgraph_store::JobState::Completed {
        engine.revision_for_job(job_id, principal, &policy)?;
    }
    Ok(progress)
}

/// 建立取消和结果共享状态并在线程执行扫描闭包。
/// 参数：work 接收取消标志与进度回调，回调绑定实时作业授权。
/// 返回：可取消和轮询的共享句柄；worker 恐慌记录为失败。
pub(crate) fn try_spawn_job(
    work: impl FnOnce(
        &Arc<AtomicBool>,
        &dyn Fn(Value, std::sync::Arc<diskgraph_engine::Engine>),
    ) -> ApiResult
    + Send
    + 'static,
) -> Result<std::sync::Arc<JobHandle>, crate::NativeServiceError> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let state = std::sync::Arc::new(std::sync::Mutex::new(JobState {
        finished: false,
        result: None,
        progress: None,
        authorization: None,
    }));
    let worker_cancel = cancel.clone();
    let worker_state = state.clone();
    let (start_tx, start_rx) = std::sync::mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("diskgraph-native-scan".into())
        .spawn(move || {
            if start_rx.recv().is_err() {
                return;
            }
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(&worker_cancel, &|value, engine| {
                    let mut state = worker_state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if state.authorization.is_none()
                        && let Some(job_id) = value["job_id"].as_str().map(str::to_owned)
                    {
                        state.authorization = Some(std::sync::Arc::new(move || {
                            authorized_job_progress(&engine, &job_id).map(|_| ())
                        }));
                    }
                    state.progress = Some(value);
                })
            }))
            .unwrap_or_else(|_| Err("scan worker crashed".into()));
            let mut state = worker_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.result = Some(outcome);
            state.finished = true;
        })
        .map_err(|error| crate::NativeServiceError::Unavailable {
            reason: format!("scan worker spawn failed: {error}"),
        })?;
    Ok(std::sync::Arc::new(JobHandle {
        cancel,
        state,
        worker: std::sync::Arc::new(crate::native_worker::NativeWorker::with_start(
            worker, start_tx,
        )),
    }))
}

/// 保留旧无错误返回的内部/UniFFI 包装，线程创建失败仍遵循原 spawn 恐慌行为。
/// 参数：work 为真实工作闭包；返回：原共享句柄，不改变旧 ABI。
pub(crate) fn spawn_job(
    work: impl FnOnce(
        &Arc<AtomicBool>,
        &dyn Fn(Value, std::sync::Arc<diskgraph_engine::Engine>),
    ) -> ApiResult
    + Send
    + 'static,
) -> std::sync::Arc<JobHandle> {
    let handle = try_spawn_job(work).expect("native scan worker spawn failed");
    handle.worker.start();
    handle
}

/// The worker body: the synchronous scan, run to completion unless the
/// cancel flag is observed before the scan starts.
/// 经 Engine 路径执行可取消的同步扫描。
/// 参数：database_path 为图库，root_path 为根目录，cancel 为调用者取消标志。
/// 返回：完成快照摘要或取消、执行错误。
pub(crate) fn run_scan_with_cancel(
    database_path: &str,
    root_path: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<serde_json::Value, String> {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("cancelled before it started".into());
    }
    // The engine's own scan bridge carries cancellation between batches;
    // the FFI layer drives it through the same engine path the server uses
    // so a cancelled job never half-publishes.
    let host = crate::native_scan_host::NativeScanHost::open(database_path, cancel, 1)?;
    // 旧同步入口只有借用标志；保留原轮询镜像，不冒称这是 JobHandle 的原 Arc。
    let request = Arc::new(AtomicBool::new(cancel.load(Ordering::SeqCst)));
    host.execute(|engine| {
        run_scan_coordinator(engine, root_path, &request, Some(cancel), &|_, _| {})
    })
}

/// 复用 Engine 驱动持久扫描作业及进度授权。
/// 参数：engine 为共享引擎，root_path 为根目录，cancel 为取消标志，progress 为进度通知。
/// 返回：完成 revision、snapshot_id、数量和覆盖范围，或失败原因。
pub(crate) fn run_scan_on_engine(
    engine: Arc<Engine>,
    root_path: &str,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(Value, Arc<Engine>),
) -> ApiResult {
    run_scan_coordinator(engine, root_path, cancel, None, progress)
}

fn run_scan_coordinator(
    engine: Arc<Engine>,
    root_path: &str,
    cancel: &Arc<AtomicBool>,
    borrowed_cancel: Option<&AtomicBool>,
    progress: &dyn Fn(Value, Arc<Engine>),
) -> ApiResult {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("cancelled before it started".into());
    }
    let principal = local_principal()?;
    let authorizer = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    let scope = engine
        .register_scope(std::path::Path::new(root_path), &principal, &authorizer)
        .map_err(|error| error.to_string())?;
    // Scope-local grants exist only after registration, so the authorizer
    // must be reloaded before the job is submitted.
    let authorizer = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    let job = engine
        .index_scope(&scope, &principal, &authorizer)
        .map_err(|error| error.to_string())?;
    progress(
        serde_json::json!({"job_id":job.job_id,"state":job.state,"observed":null}),
        engine.clone(),
    );
    // 原请求 Arc 直接传入本机 runner；成功 claim 才取得写入该 owner/fence 的取消资格。
    let deny_stop = Arc::new(AtomicBool::new(false));
    #[cfg(test)]
    let runner_hook = crate::native_runner_exit_tests::take_runner_hook();
    let mut runner = NativeRunnerGuard::spawn(
        engine.clone(),
        job.job_id.clone(),
        cancel.clone(),
        deny_stop.clone(),
        #[cfg(test)]
        runner_hook.clone(),
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    let finished = loop {
        if borrowed_cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            cancel.store(true, Ordering::SeqCst);
        }
        match authorized_progress(&engine, &job.job_id, &principal) {
            Ok(value) => progress(value, engine.clone()),
            Err(error) => {
                // 只有明确类型化拒权才设置 deny；其他 I/O/owner/预算错误原样返回。
                if matches!(
                    error,
                    EngineError::Business(BusinessError::PermissionDenied)
                ) {
                    deny_stop.store(true, Ordering::SeqCst);
                    #[cfg(test)]
                    if let Some(hook) = &runner_hook {
                        hook("denial_observed");
                    }
                }
                progress(Value::Null, engine.clone());
                return Err(progress_error(error));
            }
        }
        if runner.is_finished() {
            accept_runner_outcome(runner.join()?, cancel)?;
        }
        // 认领冲突时没有本代资格：取消本请求即可，不按任务 ID 停止他人的活代次。
        if cancel.load(Ordering::SeqCst) && !runner.is_pending() {
            return Err("the scan was cancelled by the caller".into());
        }
        #[cfg(test)]
        if let Some(hook) = &runner_hook {
            hook("after_runner_inspection");
        }
        let current = engine
            .job_status(&job.job_id)
            .map_err(|error| error.to_string())?;
        if matches!(
            current.state,
            diskgraph_store::JobState::Completed
                | diskgraph_store::JobState::Failed
                | diskgraph_store::JobState::Cancelled
        ) {
            break current.state;
        }
        if std::time::Instant::now() >= deadline {
            return Err("scan wait deadline exceeded".into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as u64;
        if !runner.is_pending()
            && (current.state == diskgraph_store::JobState::Queued
                || current.lease_expires_unix_ms <= now)
        {
            runner = NativeRunnerGuard::spawn(
                engine.clone(),
                job.job_id.clone(),
                cancel.clone(),
                deny_stop.clone(),
                #[cfg(test)]
                runner_hook.clone(),
            )?;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    // 数据库终态不代表本机 runner 已退出，正常结果也必须收取其真实线程。
    accept_runner_outcome(runner.join()?, cancel)?;
    match finished {
        diskgraph_store::JobState::Completed => {
            let authorizer = engine
                .policy_authorizer()
                .map_err(|error| error.to_string())?;
            let revision = engine
                .revision_for_job(&job.job_id, &principal, &authorizer)
                .map_err(|error| error.to_string())?;
            let snapshot = engine
                .revision_snapshot(&revision)
                .map_err(|error| error.to_string())?;
            let node_count = engine
                .revision_reader()
                .map_err(|error| error.to_string())?
                .node_count(&snapshot.id)
                .map_err(|error| error.to_string())?;
            engine
                .revision_for_job(
                    &job.job_id,
                    &principal,
                    &engine
                        .policy_authorizer()
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({
                "revision": revision,
                "snapshot_id": snapshot.id,
                "node_count": node_count,
                "coverage": snapshot.coverage,
            }))
        }
        other => Err(format!("scan ended as {other:?}")),
    }
}

fn accept_runner_outcome(
    outcome: Option<Result<diskgraph_store::JobRecord, EngineError>>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    match outcome {
        Some(Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner))) => Ok(()),
        Some(Err(error)) => {
            if cancel.load(Ordering::SeqCst) {
                Err("the scan was cancelled by the caller".into())
            } else {
                Err(error.to_string())
            }
        }
        Some(Ok(_)) | None => Ok(()),
    }
}

#[cfg(test)]
#[path = "engine_error_cleanup_tests.rs"]
mod engine_error_cleanup_tests;
