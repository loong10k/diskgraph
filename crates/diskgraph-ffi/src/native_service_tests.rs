use super::*;
use serde_json::Value;

#[test]
fn legacy_service_scan_does_not_restore_revoked_admin_grant() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"protected").unwrap();
    let service = NativeService::new(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let principal = local_principal().unwrap();
    service
        .engine
        .control_store()
        .unwrap()
        .revoke_grant(
            &principal,
            &diskgraph_core::Permission::ScopeAdmin,
            &diskgraph_engine::admin_scope(),
        )
        .unwrap();
    let handle = service
        .spawn_scan(root.path().to_str().unwrap().to_owned())
        .unwrap();
    let answer: Value = serde_json::from_str(&handle.result_json()).unwrap();
    let control = service.engine.control_store().unwrap();
    assert!(
        control.list_scopes().unwrap().is_empty(),
        "worker restored scope registration rights"
    );
    assert!(control.list_queued_jobs().unwrap().is_empty());
    drop(control);
    assert_eq!(
        answer["ok"], false,
        "revoked administrator scanned: {answer}"
    );
    assert!(
        answer["error"]
            .as_str()
            .unwrap()
            .contains("permission_denied"),
        "{answer}"
    );
    service.shutdown();
}

#[test]
fn persistent_service_queries_deny_revoked_scope_and_closed_session() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"data").unwrap();
    let path = data
        .path()
        .join("graph.sqlite")
        .to_string_lossy()
        .into_owned();
    let scan: Value = serde_json::from_str(&scan_native_json(
        path.clone(),
        root.path().to_string_lossy().into_owned(),
    ))
    .unwrap();
    let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
    let service = NativeService::new(path).unwrap();
    let answer: Value = serde_json::from_str(&service.node_json(snapshot.clone(), 1)).unwrap();
    assert_eq!(answer["ok"], true);
    let policy = service.engine.policy_authorizer().unwrap();
    let revision = scan["data"]["revision"].as_str().unwrap();
    let scope = service
        .engine
        .authorize_revision(None, revision, &local_principal().unwrap(), &policy)
        .unwrap();
    service
        .engine
        .revoke_scope(&scope, &local_principal().unwrap(), &policy)
        .unwrap();
    let denied: Value = serde_json::from_str(&service.node_json(snapshot.clone(), 1)).unwrap();
    assert_eq!(denied["ok"], false);
    service.shutdown();
    let closed: Value = serde_json::from_str(&service.node_json(snapshot, 1)).unwrap();
    assert_eq!(closed["ok"], false);
    assert!(closed["error"].as_str().unwrap().contains("closed"));
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn session_children_clip_bytes_and_advance_by_actual_page_size() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let leaf = root
        .path()
        .join("x".repeat(180))
        .join("y".repeat(180))
        .join("z".repeat(180));
    std::fs::create_dir_all(&leaf).unwrap();
    for n in 0..100 {
        std::fs::write(leaf.join(format!("file-{n:03}")), b"x").unwrap();
    }
    let path = data
        .path()
        .join("graph.sqlite")
        .to_string_lossy()
        .into_owned();
    let scan: Value = serde_json::from_str(&scan_native_json(
        path.clone(),
        root.path().to_string_lossy().into_owned(),
    ))
    .unwrap();
    let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
    let db = rusqlite::Connection::open(&path).unwrap();
    let parent: i64 = db
        .query_row(
            "SELECT id FROM nodes WHERE name=?1",
            ["z".repeat(180)],
            |row| row.get(0),
        )
        .unwrap();
    let service = NativeService::new(path).unwrap();
    let text = service.children_json(snapshot, parent as u64, 0, 100);
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer["ok"], true, "{answer}");
    assert!(text.len() <= diskgraph_core::QueryBudget::default().max_response_bytes);
    let count = answer["data"]["items"].as_array().unwrap().len();
    assert!(count < 100);
    assert_eq!(answer["data"]["next_offset"], count as u64);
    assert_eq!(answer["data"]["complete"], false);
}

#[test]
fn two_native_handles_for_a_coalesced_job_return_the_same_revision() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    for n in 0..2000 {
        std::fs::write(root.path().join(format!("file-{n}")), b"x").unwrap();
    }
    let service = NativeService::new(
        data.path()
            .join("graph.sqlite")
            .to_string_lossy()
            .into_owned(),
    )
    .unwrap();
    let first = service
        .spawn_scan(root.path().to_string_lossy().into_owned())
        .unwrap();
    let second = service
        .spawn_scan(root.path().to_string_lossy().into_owned())
        .unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "coalesced subscriptions must share last-handle cancellation"
    );
    let left: Value = serde_json::from_str(&first.result_json()).unwrap();
    let right: Value = serde_json::from_str(&second.result_json()).unwrap();
    assert_eq!(left["ok"], true, "{left}");
    assert_eq!(right["ok"], true, "{right}");
    assert_eq!(left["data"]["revision"], right["data"]["revision"]);
}

#[test]
fn completed_native_job_handle_does_not_keep_revoked_permissions() {
    for permission in [
        diskgraph_core::Permission::OperationView,
        diskgraph_core::Permission::MetadataRead,
    ] {
        revoked_job_handle(permission, false);
    }
}

#[test]
fn active_native_job_handle_observes_individual_grant_revocation() {
    for permission in [
        diskgraph_core::Permission::OperationView,
        diskgraph_core::Permission::MetadataRead,
    ] {
        revoked_job_handle(permission, true);
    }
}

fn revoked_job_handle(permission: diskgraph_core::Permission, active: bool) {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let path = data
        .path()
        .join("graph.sqlite")
        .to_string_lossy()
        .into_owned();
    let service = NativeService::new(path).unwrap();
    let (mut gate, hook) = crate::native_scan_gate::NativeScanGate::new();
    service.set_scan_progress_hook(hook);
    let handle = service
        .spawn_scan(root.path().to_string_lossy().into_owned())
        .unwrap();
    let principal = local_principal().unwrap();
    let queued = gate.wait_queued();
    let job_id = queued["job_id"].as_str().unwrap().to_owned();
    assert_eq!(queued["state"], "queued", "{queued}");
    assert_eq!(
        service.engine.job_status(&job_id).unwrap().state,
        diskgraph_store::JobState::Queued,
        "synchronization must observe a real persisted queued job"
    );
    let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
    assert_eq!(progress["ok"], true, "{progress}");
    assert_eq!(progress["data"]["progress"]["job_id"], job_id);
    if active {
        assert!(
            handle.poll_result_json().is_none(),
            "fixture revokes an active FFI handle; durable scan is still queued"
        );
    } else {
        gate.release();
        let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(
            service.engine.job_status(&job_id).unwrap().state,
            diskgraph_store::JobState::Completed
        );
    }
    let job = service.engine.job_status(&job_id).unwrap();
    let scope = job.scope_id;
    let stale_policy = service.engine.policy_authorizer().unwrap();
    let mut control = service.engine.control_store().unwrap();
    control
        .revoke_grant(&principal, &permission, &scope)
        .unwrap();
    drop(control);
    if permission == diskgraph_core::Permission::OperationView {
        assert!(
            service
                .engine
                .job_progress(&job_id, &principal, &stale_policy)
                .is_err(),
            "Engine must intersect stale capability with live grant"
        );
    } else if !active {
        assert!(
            service
                .engine
                .revision_for_job(&job_id, &principal, &stale_policy)
                .is_err()
        );
    }
    let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
    assert_eq!(progress["ok"], false);
    // 即使真实 worker 尚未返回，poll 也必须立即应用刚撤销的授权。
    let pending: Value = serde_json::from_str(&handle.poll_result_json().unwrap()).unwrap();
    assert_eq!(pending["ok"], false, "{pending}");
    gate.release();
    let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
    assert_eq!(
        result["ok"], false,
        "cached result bypassed current permissions: {result}"
    );
    let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
    assert_eq!(progress["ok"], false);
    let polled: Value = serde_json::from_str(&handle.poll_result_json().unwrap()).unwrap();
    assert_eq!(polled["ok"], false);
    // 独立的清理 watchdog；不把准备时间算作后台 owner 的退出预算。
    let cleanup_deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !matches!(
        service.engine.job_status(&job_id).unwrap().state,
        diskgraph_store::JobState::Completed
            | diskgraph_store::JobState::Cancelled
            | diskgraph_store::JobState::Failed
    ) {
        assert!(
            std::time::Instant::now() < cleanup_deadline,
            "background owner must terminate before fixture cleanup"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn last_job_handle_drop_requests_cancel_and_poll_never_joins() {
    let (release, waiting) = std::sync::mpsc::channel();
    let handle = crate::spawn_job(move |_, _| {
        waiting
            .recv_timeout(std::time::Duration::from_secs(60))
            .unwrap();
        Ok(serde_json::json!({"released":true}))
    });
    let flag = handle.cancel.clone();
    let worker = handle.worker.clone();
    assert!(handle.poll_result_json().is_none());
    drop(handle);
    assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    release.send(()).unwrap();
    worker.join().unwrap();
}
