//! 扫描 worker、持久作业轮询和取消信号协调，保持 Engine 为执行主体。
use crate::JobHandle;
use crate::api_result::ApiResult;
use crate::job_state::JobState;
use crate::native_realm::{local_principal, open_engine};
use serde_json::Value;

fn authorized_job_progress(engine: &diskgraph_engine::Engine, job_id: &str) -> ApiResult {
    let principal = local_principal()?;
    let policy = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    let progress = engine
        .job_progress(job_id, &principal, &policy)
        .map_err(|error| error.to_string())?;
    let job = engine
        .job_status(job_id)
        .map_err(|error| error.to_string())?;
    if engine
        .control_store()
        .map_err(|error| error.to_string())?
        .live_permission(
            &principal,
            &diskgraph_core::Permission::MetadataRead,
            &job.scope_id,
        )
        .map_err(|error| error.to_string())?
        == Some(false)
    {
        return Err("permission_denied".into());
    }
    if job.state == diskgraph_store::JobState::Completed {
        engine
            .revision_for_job(job_id, &principal, &policy)
            .map_err(|error| error.to_string())?;
    }
    Ok(progress)
}

/// 建立取消和结果共享状态并在线程执行扫描闭包。
/// 参数：work 接收取消标志与进度回调，回调绑定实时作业授权。
/// 返回：可取消和轮询的共享句柄；worker 恐慌记录为失败。
pub(crate) fn spawn_job(
    work: impl FnOnce(
        &std::sync::atomic::AtomicBool,
        &dyn Fn(Value, std::sync::Arc<diskgraph_engine::Engine>),
    ) -> ApiResult
    + Send
    + 'static,
) -> std::sync::Arc<JobHandle> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let state = std::sync::Arc::new(std::sync::Mutex::new(JobState {
        finished: false,
        result: None,
        progress: None,
        authorization: None,
    }));
    let worker_cancel = cancel.clone();
    let worker_state = state.clone();
    std::thread::spawn(move || {
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
    });
    std::sync::Arc::new(JobHandle { cancel, state })
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
    let engine = std::sync::Arc::new(open_engine(database_path)?);
    run_scan_on_engine(engine, root_path, cancel, &|_, _| {})
}

/// 复用 Engine 驱动持久扫描作业及进度授权。
/// 参数：engine 为共享引擎，root_path 为根目录，cancel 为取消标志，progress 为进度通知。
/// 返回：完成 revision、snapshot_id、数量和覆盖范围，或失败原因。
pub(crate) fn run_scan_on_engine(
    engine: std::sync::Arc<diskgraph_engine::Engine>,
    root_path: &str,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &dyn Fn(Value, std::sync::Arc<diskgraph_engine::Engine>),
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
    // The job is claimed and executed on its own thread, while this worker
    // polls: it mirrors the FFI cancel flag into the engine's cancellation
    // channel and collects the final state.
    let runner_engine = std::sync::Arc::clone(&engine);
    let runner_job = job.job_id.clone();
    let mut runner = Some(std::thread::spawn(move || {
        runner_engine.run_job(&runner_job, "ffi-worker")
    }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    let finished = loop {
        let authorizer = engine
            .policy_authorizer()
            .map_err(|error| error.to_string())?;
        let value = authorized_job_progress(&engine, &job.job_id);
        match value {
            Ok(value) => progress(value, engine.clone()),
            Err(error) => {
                progress(Value::Null, engine.clone());
                let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
                return Err(error);
            }
        }
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
        }
        if runner.as_ref().is_some_and(|thread| thread.is_finished()) {
            let outcome = runner
                .take()
                .unwrap()
                .join()
                .map_err(|_| "scan worker crashed".to_owned())?;
            match outcome {
                // 同 scope 请求共享 durable job；认领冲突只表示另一个活 owner 正在完成它。
                Err(diskgraph_engine::EngineError::Store(
                    diskgraph_store::StoreError::StaleOwner,
                )) => {}
                Err(error) => {
                    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err("the scan was cancelled by the caller".into());
                    }
                    return Err(error.to_string());
                }
                Ok(_) => {}
            }
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
            let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
            return Err("scan wait deadline exceeded".into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as u64;
        if runner.is_none()
            && (current.state == diskgraph_store::JobState::Queued
                || current.lease_expires_unix_ms <= now)
        {
            let claiming = engine.clone();
            let id = job.job_id.clone();
            runner = Some(std::thread::spawn(move || {
                claiming.run_job(&id, "ffi-worker")
            }));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
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
