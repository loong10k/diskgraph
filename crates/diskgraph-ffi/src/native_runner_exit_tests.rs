//! PF-06 内层实际 Engine runner：持久终态、权限错误和 unwind 均不能遗弃其 JoinHandle。
//! 旧实现的裸句柄会丢弃；RED 只观察提前 result，不宣称已经 join 旧 runner。
use crate::native_worker_exit_barrier::{NativeWorkerExitBarrier, serialize_tls_fixture};
use crate::{NativeService, local_principal};
use diskgraph_core::Permission;
use diskgraph_store::JobState;
use serde_json::Value;
use std::cell::RefCell;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::channel,
};
use std::thread;
use std::time::Duration;

/// 请求局部真实 runner 阶段观察；来源：DiskGraph 原生 Rust FFI 线程生命周期回归。
pub(crate) type RunnerHook = Arc<dyn Fn(&str) + Send + Sync>;

thread_local! {
    static RUNNER_HOOK: RefCell<Option<RunnerHook>> = const { RefCell::new(None) };
}

/// 从当前外层协调线程取得测试请求阶段观察。参数：无；返回：单次真实阶段钩子。
pub(crate) fn take_runner_hook() -> Option<RunnerHook> {
    RUNNER_HOOK.with(|slot| slot.borrow_mut().take())
}

/// 为当前真实协调请求安装测试阶段观察。参数：hook 为共享回调；返回：无。
pub(crate) fn install_runner_hook(hook: RunnerHook) {
    RUNNER_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[test]
fn completed_result_waits_for_actual_inner_runner_tls_exit() {
    inner_runner_exit("completed");
}

#[test]
fn late_permission_denial_waits_for_runner_without_rewriting_committed_fact() {
    inner_runner_exit("denied");
}

#[test]
fn coordinator_unwind_does_not_abandon_its_actual_inner_runner() {
    inner_runner_exit("unwind");
}

fn inner_runner_exit(mode: &'static str) {
    let _fixture = serialize_tls_fixture();
    let data = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("ordinary"), b"real inner runner").unwrap();
    let service = NativeService::new(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let (job_tx, job_rx) = channel();
    let (outcome_tx, outcome_rx) = channel();
    let (return_tx, return_rx) = channel();
    let (returned_tx, returned_rx) = channel();
    let (inspection_tx, inspection_rx) = channel();
    let (inspect_release_tx, inspect_release_rx) = channel();
    let (inspection_released_tx, inspection_released_rx) = channel();
    let (tls_tx, tls_rx) = channel();
    let (tls_release_tx, tls_release_rx) = channel();
    let (tls_released_tx, tls_released_rx) = channel();
    let returns = Mutex::new(return_rx);
    let inspections = Mutex::new(inspect_release_rx);
    let tls = Mutex::new(Some((tls_tx, tls_release_rx, tls_released_tx)));
    let inspected = AtomicBool::new(false);
    let hook: RunnerHook = Arc::new(move |stage| match stage {
        "runner_start" => {
            let (reached, release, released) = tls.lock().unwrap().take().unwrap();
            NativeWorkerExitBarrier::install(reached, release, released);
        }
        "runner_outcome" => {
            let _ = outcome_tx.send(());
            let active = returns
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(60))
                .is_ok();
            let _ = returned_tx.send(active);
        }
        "after_runner_inspection" if !inspected.swap(true, Ordering::SeqCst) => {
            let _ = inspection_tx.send(());
            let active = inspections
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(60))
                .is_ok();
            let _ = inspection_released_tx.send(active);
            if mode == "unwind" {
                panic!("真实协调线程在持有 inner runner 时 unwind");
            }
        }
        _ => {}
    });
    let installed = AtomicBool::new(false);
    service.set_scan_progress_hook(Arc::new(move |value| {
        if !installed.swap(true, Ordering::SeqCst) {
            RUNNER_HOOK.with(|slot| *slot.borrow_mut() = Some(hook.clone()));
            let _ = job_tx.send(value["job_id"].as_str().unwrap().to_owned());
        }
    }));
    let handle = service
        .spawn_scan(root.path().to_str().unwrap().to_owned())
        .unwrap();
    let job_id = job_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    let inspected = inspection_rx.recv_timeout(Duration::from_secs(60));
    let engine_returned = outcome_rx.recv_timeout(Duration::from_secs(60));
    // 上述门不持数据库或 JobState 锁；读取真实提交事实后才选择撤权。
    let committed = service.engine.job_status(&job_id).unwrap();
    let principal = local_principal().unwrap();
    let policy = service.engine.policy_authorizer().unwrap();
    let revision = service
        .engine
        .revision_for_job(&job_id, &principal, &policy)
        .unwrap();
    if mode == "denied" {
        service
            .engine
            .control_store()
            .unwrap()
            .revoke_grant(&principal, &Permission::MetadataRead, &committed.scope_id)
            .unwrap();
    }
    // runner 仍停在普通返回门；先确认 reader 已启动，再允许真实 TLS 析构开始。
    let (reader_ready_tx, reader_ready_rx) = channel();
    let (reader_start_tx, reader_start_rx) = channel();
    let (calling_tx, calling_rx) = channel();
    let (result_tx, result_rx) = channel();
    let reader_handle = handle.clone();
    let reader = thread::spawn(move || {
        let _ = reader_ready_tx.send(());
        reader_start_rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap();
        let _ = calling_tx.send(());
        let result = reader_handle.result_json();
        let _ = result_tx.send(result.clone());
        result
    });
    let reader_ready = reader_ready_rx.recv_timeout(Duration::from_secs(5));
    // coordinator 已检查过尚未返回的 runner；现在放其进入实际 TLS，再放终态分支。
    let return_sent = return_tx.send(());
    let returned = returned_rx.recv_timeout(Duration::from_secs(10));
    let tls_reached = tls_rx.recv_timeout(Duration::from_secs(10));
    let reader_started = reader_start_tx.send(());
    let calling = calling_rx.recv_timeout(Duration::from_secs(5));
    let inspection_release = inspect_release_tx.send(());
    let inspection_released = inspection_released_rx.recv_timeout(Duration::from_secs(10));
    let early_result = result_rx.recv_timeout(Duration::from_millis(100));
    let tls_release = tls_release_tx.send(());
    let tls_released = tls_released_rx.recv_timeout(Duration::from_secs(10));
    let result = reader.join();
    let final_job = service.engine.job_status(&job_id).unwrap();
    let latest = service.engine.latest_revision(&committed.scope_id).unwrap();
    service.shutdown();

    eprintln!(
        "inner runner mode={mode}, early_result={early_result:?}, tls_released={tls_released:?}"
    );
    assert!(
        inspected.is_ok() && engine_returned.is_ok(),
        "阶段未到达: {inspected:?}/{engine_returned:?}"
    );
    assert!(
        reader_ready.is_ok(),
        "reader 必须在 TLS 之前启动: {reader_ready:?}"
    );
    assert_eq!(committed.state, JobState::Completed);
    assert_eq!(committed.owner.as_str(), "ffi-worker");
    assert!(committed.fencing_token > 0);
    assert!(
        reader_started.is_ok() && calling.is_ok(),
        "真实 result 入口未到达: {calling:?}"
    );
    assert!(return_sent.is_ok() && returned.unwrap());
    assert!(tls_reached.is_ok() && inspection_release.is_ok() && inspection_released.unwrap());
    assert!(
        tls_release.is_ok() && tls_released.unwrap(),
        "TLS 必须主动释放，不能等救援"
    );
    assert_eq!(final_job.state, JobState::Completed);
    assert_eq!(final_job.owner, committed.owner);
    assert_eq!(final_job.fencing_token, committed.fencing_token);
    assert_eq!(latest.as_deref(), Some(revision.as_str()));
    let result: Value = serde_json::from_str(&result.unwrap()).unwrap();
    if mode == "completed" {
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["revision"], revision);
    } else if mode == "denied" {
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(result["error"], "permission_denied", "{result}");
    } else {
        assert_eq!(result["ok"], false, "{result}");
        assert!(
            result.to_string().contains("scan worker crashed"),
            "{result}"
        );
    }
    assert!(
        matches!(
            early_result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "真实 inner runner 的 TLS 尚未退出，result 却提前返回: {early_result:?}"
    );
}
