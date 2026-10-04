//! PF-06 准入回归：用真实 spawn_scan 与请求局部门证明关闭先于后续路径 I/O。
use crate::{NativeService, NativeServiceError};
use std::cell::RefCell;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

thread_local! {
    static BEFORE_CANONICALIZE: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// 调用当前测试请求的单次路径阶段同步点；来源：Rust 原生 NativeService 测试。
/// 参数：无，回调只能由当前测试线程安装；返回：无，不改变生产构建。
pub(crate) fn before_canonicalize() {
    let hook = BEFORE_CANONICALIZE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

fn assert_closed(result: Result<std::sync::Arc<crate::JobHandle>, NativeServiceError>) {
    match result {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "session closed", "不得以路径错误替代关闭拒绝");
        }
        Ok(_) => panic!("关闭后仍返回扫描句柄"),
    }
}

#[test]
fn closed_service_refuses_scan_before_resolving_a_missing_path() {
    let data = tempfile::tempdir().unwrap();
    let service = NativeService::new(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let missing = data.path().join("never-created");
    assert!(!missing.exists());
    service.shutdown();
    let result = service.spawn_scan(missing.to_str().unwrap().to_owned());
    assert!(
        service
            .engine
            .control_store()
            .unwrap()
            .list_scopes()
            .unwrap()
            .is_empty()
    );
    assert!(service.engine.queued_jobs().unwrap().is_empty());
    assert_closed(result);
}

#[test]
fn close_during_admission_refuses_path_work_and_never_enqueues() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = source.path().join("scope");
    std::fs::create_dir(&root).unwrap();
    let service = NativeService::new(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let worker_service = service.clone();
    let requested_root = root.to_str().unwrap().to_owned();
    let worker = thread::spawn(move || {
        BEFORE_CANONICALIZE.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let _ = entered_tx.send(());
                let released = release_rx.recv_timeout(Duration::from_secs(60)).is_ok();
                let _ = released_tx.send(released);
            }));
        });
        worker_service.spawn_scan(requested_root)
    });
    let entered = entered_rx.recv_timeout(Duration::from_secs(10));
    service.shutdown();
    // 只删除本测试自建的空目录；若仍执行 canonicalize，将得到真实不存在错误。
    let removed = std::fs::remove_dir(&root);
    let release_sent = release_tx.send(());
    let released = released_rx.recv_timeout(Duration::from_secs(10));
    let result = worker.join();

    assert!(entered.is_ok(), "真实请求未到达路径阶段: {entered:?}");
    assert!(removed.is_ok(), "隔离目录未成功删除: {removed:?}");
    assert!(release_sent.is_ok());
    assert!(
        released.unwrap(),
        "门必须主动释放，不能以 watchdog 退出充数"
    );
    assert!(
        service
            .engine
            .control_store()
            .unwrap()
            .list_scopes()
            .unwrap()
            .is_empty()
    );
    assert!(service.engine.queued_jobs().unwrap().is_empty());
    assert_closed(result.unwrap());
}

#[test]
fn all_in_flight_admissions_are_bounded_and_remain_pending_after_close() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let (service, mut owner) = NativeService::new_with_owner(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let limit = diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
    let (entered_tx, entered_rx) = channel();
    let (released_tx, released_rx) = channel();
    let mut releases = Vec::new();
    let mut threads = Vec::new();
    for _ in 0..limit {
        let service = service.clone();
        let path = source.path().to_str().unwrap().to_owned();
        let entered = entered_tx.clone();
        let released = released_tx.clone();
        let (release_tx, release_rx) = channel();
        releases.push(release_tx);
        threads.push(thread::spawn(move || {
            BEFORE_CANONICALIZE.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    let _ = entered.send(());
                    let resumed = release_rx.recv_timeout(Duration::from_secs(60)).is_ok();
                    let _ = released.send(resumed);
                }));
            });
            service.spawn_scan(path)
        }));
    }
    let entered: Vec<_> = (0..limit)
        .map(|_| entered_rx.recv_timeout(Duration::from_secs(10)))
        .collect();
    let overflow = service.spawn_scan(source.path().to_str().unwrap().to_owned());
    let pending =
        service.drain_coordinators_until(std::time::Instant::now() + Duration::from_millis(30));
    for release in releases {
        let _ = release.send(());
    }
    let released: Vec<_> = (0..limit)
        .map(|_| released_rx.recv_timeout(Duration::from_secs(10)))
        .collect();
    let results: Vec<_> = threads.into_iter().map(thread::JoinHandle::join).collect();
    let drained =
        service.drain_coordinators_until(std::time::Instant::now() + Duration::from_secs(10));

    assert!(
        entered.iter().all(Result::is_ok),
        "所有真实路径阶段必须命中: {entered:?}"
    );
    assert!(released.into_iter().all(|released| released == Ok(true)));
    match overflow {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "resource_exhausted: native admissions")
        }
        Ok(_) => panic!("超出额度的准入启动了路径解析/工作线程"),
    }
    match pending {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "coordinator drain deadline exceeded")
        }
        Ok(()) => panic!("registry 尚无线程不表示已经登记的路径请求退出"),
    }
    for result in results {
        assert_closed(result.unwrap());
    }
    drained.unwrap();
    assert!(
        service
            .engine
            .control_store()
            .unwrap()
            .list_scopes()
            .unwrap()
            .is_empty()
    );
    assert!(service.engine.queued_jobs().unwrap().is_empty());
    owner.finalize_owner().unwrap();
}

#[test]
fn path_failure_and_admission_unwind_return_the_original_quota() {
    let data = tempfile::tempdir().unwrap();
    let (service, mut owner) = NativeService::new_with_owner(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let missing = data.path().join("not-created");
    BEFORE_CANONICALIZE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(|| panic!("真实准入后 unwind 夹具")));
    });
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        service.spawn_scan(missing.to_str().unwrap().to_owned())
    }));
    assert!(panicked.is_err());
    let expected = missing.canonicalize().unwrap_err().to_string();
    let limit = diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
    for _ in 0..(limit * 2 + 1) {
        match service.spawn_scan(missing.to_str().unwrap().to_owned()) {
            Err(NativeServiceError::Unavailable { reason }) => assert_eq!(reason, expected),
            Ok(_) => panic!("不存在的路径不能启动任务"),
        }
    }
    service
        .drain_coordinators_until(std::time::Instant::now() + Duration::from_secs(10))
        .unwrap();
    assert!(service.engine.queued_jobs().unwrap().is_empty());
    owner.finalize_owner().unwrap();
}
