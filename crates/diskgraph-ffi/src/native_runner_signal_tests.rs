//! PF-06 真实公开 FFI 请求接线：原 Arc 到 Engine 的同代取消/独立拒权，不用裸任务 ID 停止。
use crate::native_runner_exit_tests::{RunnerHook, install_runner_hook};
use crate::{NativeService, local_principal};
use diskgraph_core::Permission;
use diskgraph_store::JobState;
use serde_json::Value;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::channel,
};
use std::time::Duration;

#[test]
fn caller_cancel_reaches_the_actual_claimed_generation() {
    signal_case("cancel");
}

#[test]
fn actual_operation_view_denial_stops_runner_without_permanent_cancel() {
    signal_case("deny");
}

#[test]
fn observed_denial_wins_over_the_original_caller_cancel_signal() {
    signal_case("both");
}

#[test]
fn cancelled_request_with_failed_claim_does_not_cancel_other_live_owner() {
    signal_case("other_owner");
}

fn signal_case(mode: &'static str) {
    let data = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"real scoped runner").unwrap();
    let database = data.path().join("graph.sqlite");
    let service = NativeService::new(database.to_str().unwrap().to_owned()).unwrap();
    let (job_tx, job_rx) = channel();
    let (start_tx, start_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let (denied_tx, denied_rx) = channel();
    let release = Mutex::new(release_rx);
    let hook: RunnerHook = Arc::new(move |stage| match stage {
        "runner_start" => {
            let _ = start_tx.send(());
            let active = release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(60))
                .is_ok();
            let _ = released_tx.send(active);
        }
        "denial_observed" => {
            let _ = denied_tx.send(());
        }
        _ => {}
    });
    let installed = AtomicBool::new(false);
    service.set_scan_progress_hook(Arc::new(move |progress| {
        if !installed.swap(true, Ordering::SeqCst) {
            install_runner_hook(hook.clone());
            let _ = job_tx.send(progress["job_id"].as_str().unwrap().to_owned());
        }
    }));
    let handle = service
        .spawn_scan(root.path().to_str().unwrap().to_owned())
        .unwrap();
    let job_id = job_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    let started = start_rx.recv_timeout(Duration::from_secs(60));
    let queued = service.engine.job_status(&job_id).unwrap();
    let principal = local_principal().unwrap();
    let denied = mode == "deny" || mode == "both";
    let other = if mode == "other_owner" {
        // 通过正式 claim API 建立另一个真实活代次，不伪造控制库 owner/fence。
        Some(
            service
                .engine
                .control_store()
                .unwrap()
                .claim_job_once(&job_id, "different-native-host")
                .unwrap(),
        )
    } else {
        None
    };
    if denied {
        service
            .engine
            .control_store()
            .unwrap()
            .revoke_grant(&principal, &Permission::OperationView, &queued.scope_id)
            .unwrap();
    }
    if mode != "deny" {
        handle.cancel();
    }
    // 实际权限检查已经失败且独立 deny 信号已经设置，再放行原请求的真实 claim。
    let denial_observed = if denied {
        Some(denied_rx.recv_timeout(Duration::from_secs(10)))
    } else {
        None
    };
    let release_sent = release_tx.send(());
    let released = released_rx.recv_timeout(Duration::from_secs(10));
    let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
    let final_job = service.engine.job_status(&job_id).unwrap();
    let cancelled = service
        .engine
        .control_store()
        .unwrap()
        .cancellation_requested(&job_id, final_job.fencing_token)
        .unwrap();
    let latest = service.engine.latest_revision(&queued.scope_id).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    let snapshots: i64 = connection
        .query_row("SELECT count(*) FROM snapshots", [], |row| row.get(0))
        .unwrap();
    let revisions: i64 = connection
        .query_row("SELECT count(*) FROM graph_revisions", [], |row| row.get(0))
        .unwrap();
    if let Some(other) = &other {
        // 只在断言资料已取出后，用拥有者真实 fence 清理本测试主动认领的代次。
        service
            .engine
            .control_store()
            .unwrap()
            .finish_job_fenced(
                &job_id,
                &other.owner,
                other.fencing_token,
                JobState::Cancelled,
            )
            .unwrap();
    }
    service.shutdown();

    assert!(started.is_ok());
    assert_eq!(queued.state, JobState::Queued);
    assert!(release_sent.is_ok() && released.unwrap());
    assert!(handle.worker.is_joined());
    assert_eq!(result["ok"], false, "{mode}: {result}");
    assert!(latest.is_none());
    assert_eq!((snapshots, revisions), (0, 0));
    if let Some(other) = other {
        assert_eq!(final_job.state, JobState::Running);
        assert_eq!(final_job.owner, other.owner);
        assert_eq!(final_job.fencing_token, other.fencing_token);
        assert_eq!(final_job.lease_expires_unix_ms, other.lease_expires_unix_ms);
        assert!(!cancelled, "未成功 claim 的请求不能写他人取消位");
        assert_eq!(result["error"], "the scan was cancelled by the caller");
    } else if denied {
        assert!(denial_observed.unwrap().is_ok(), "必须实际观察权限拒绝");
        assert_eq!(final_job.state, JobState::Failed);
        assert_eq!(final_job.owner, "ffi-worker");
        assert!(final_job.fencing_token > queued.fencing_token);
        assert!(!cancelled, "拒权不能持久改成用户取消");
        assert_eq!(result["error"], "permission_denied");
    } else {
        assert_eq!(final_job.state, JobState::Cancelled);
        assert_eq!(final_job.owner, "ffi-worker");
        assert!(final_job.fencing_token > queued.fencing_token);
        assert!(cancelled);
        assert_eq!(result["error"], "the scan was cancelled by the caller");
    }
}
