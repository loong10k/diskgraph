//! PF-06 作用域守卫：忘记公开借用能力不能遗弃真实 manager，正常返回和 unwind 均须回收。
use crate::NativeService;
use crate::native_service_owner_tests::install_manager_start_hook;
use crate::native_worker_exit_barrier::{NativeWorkerExitBarrier, serialize_tls_fixture};
use std::sync::{Arc, mpsc::channel};
use std::thread;
use std::time::Duration;

#[test]
fn scoped_service_scans_with_physical_host_and_closes_after_scope() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("owned.txt"), b"actual worker input").unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let (service, snapshot) = NativeService::with_owner(database, |service, _owner| {
        let job = service
            .spawn_scan(root.path().to_str().unwrap().to_owned())
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&job.result_json()).unwrap();
        assert_eq!(result["ok"], true, "actual scoped scan failed: {result}");
        let snapshot = result["data"]["snapshot_id"].as_str().unwrap().to_owned();
        let node: serde_json::Value =
            serde_json::from_str(&service.node_json(snapshot.clone(), 1)).unwrap();
        assert_eq!(node["ok"], true, "scoped query failed: {node}");
        (service, snapshot)
    })
    .unwrap();
    let closed: serde_json::Value = serde_json::from_str(&service.node_json(snapshot, 1)).unwrap();
    assert_eq!(closed["ok"], false);
    assert!(closed["error"].as_str().unwrap().contains("closed"));
}

#[test]
fn forgotten_capability_keeps_manager_owned_until_scope_returns() {
    forgotten_capability_exit(false);
}

#[test]
fn forgotten_capability_keeps_manager_owned_during_callback_unwind() {
    forgotten_capability_exit(true);
}

fn forgotten_capability_exit(panics: bool) {
    let _fixture = serialize_tls_fixture();
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let (ready_tx, ready_rx) = channel();
    let (enter_tx, enter_rx) = channel();
    let (callback_tx, callback_rx) = channel();
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let (returned_tx, returned_rx) = channel();
    // 唯一宿主先启动并发出 ready；TLS 激活后不再启动任何参与线程。
    let host = thread::spawn(move || {
        ready_tx.send(()).unwrap();
        enter_rx.recv_timeout(Duration::from_secs(60)).unwrap();
        install_manager_start_hook(Box::new(move || {
            NativeWorkerExitBarrier::install(reached_tx, release_rx, released_tx);
        }));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            NativeService::with_owner(database, |service, owner| {
                let weak = Arc::downgrade(&service);
                drop(service);
                let service_gone = weak.upgrade().is_none();
                std::mem::forget(owner);
                // 只证明回调已忘记能力；随后 TLS/真实 join 仍须由独立库栈守卫承担。
                callback_tx.send(service_gone).unwrap();
                if panics {
                    panic!("scoped host original panic");
                }
                "scoped host return marker"
            })
        }));
        let _ = returned_tx.send(());
        outcome
    });
    let ready = ready_rx.recv_timeout(Duration::from_secs(5));
    let entered = enter_tx.send(());
    let callback = callback_rx.recv_timeout(Duration::from_secs(60));
    let reached = reached_rx.recv_timeout(Duration::from_secs(10));
    let early_return = returned_rx.recv_timeout(Duration::from_millis(100));
    // 先主动救援并 join 自有宿主，再断言；接口缺失不记作运行 RED。
    let released = release_tx.send(());
    let active_release = released_rx.recv_timeout(Duration::from_secs(10));
    let joined = host.join();

    assert!(ready.is_ok() && entered.is_ok());
    assert_eq!(callback, Ok(true), "守卫不得强持已释放的 Service");
    assert!(reached.is_ok(), "真实 manager TLS 未到达: {reached:?}");
    assert!(
        matches!(
            early_return,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "忘记能力导致 with_owner 在实际 manager 退出前返回: {early_return:?}"
    );
    assert!(released.is_ok() && active_release == Ok(true));
    let outcome = joined.unwrap();
    if panics {
        let panic = outcome.expect_err("必须保留原 callback panic");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"scoped host original panic")
        );
    } else {
        assert_eq!(outcome.unwrap().unwrap(), "scoped host return marker");
    }
}

#[test]
fn escaped_service_is_closed_after_forgotten_owner_scope_finishes() {
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let service = NativeService::with_owner(database, |service, owner| {
        std::mem::forget(owner);
        service
    })
    .unwrap();
    let missing = data
        .path()
        .join("never-created")
        .to_str()
        .unwrap()
        .to_owned();
    match service.spawn_scan(missing) {
        Err(crate::NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "session closed")
        }
        Ok(_) => panic!("作用域结束后逃逸的 Service 仍接受工作"),
    }
}

#[test]
fn scope_reports_real_manager_failure_after_callback_forgot_capability() {
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let callback_ran = std::cell::Cell::new(false);
    let (started_tx, started_rx) = channel();
    install_manager_start_hook(Box::new(move || {
        started_tx.send(()).unwrap();
        panic!("actual scoped manager failure");
    }));
    let result = NativeService::with_owner(database, |service, owner| {
        drop(service);
        std::mem::forget(owner);
        callback_ran.set(true);
        "callback value cannot mask cleanup failure"
    });
    assert!(started_rx.recv_timeout(Duration::from_secs(10)).is_ok());
    assert!(callback_ran.get());
    match result {
        Err(crate::NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "coordinator join manager failed");
        }
        Ok(_) => panic!("真实 manager 失败不能作为 callback 成功返回"),
    }
}
