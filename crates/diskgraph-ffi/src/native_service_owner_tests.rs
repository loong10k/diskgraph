//! PF-06 显式 Rust 宿主 owner：真实协调线程 drain 与 manager 最终 join 分层验收。
//! 新构造接口尚缺时只记录接口缺失；旧同步 try_join 的实际 60s RED 已独立保存。
use crate::native_worker_exit_barrier::NativeWorkerExitBarrier;
use crate::{NativeService, NativeServiceError};
use serde_json::Value;
use std::cell::RefCell;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::channel,
};
use std::thread;
use std::time::{Duration, Instant};

thread_local! {
    static MANAGER_START: RefCell<Option<Box<dyn FnOnce() + Send>>> = const { RefCell::new(None) };
}

/// 取出新 manager 的请求局部启动观察，仅测试构建使用。
/// 参数：无；返回：由真实 manager 线程执行一次的 TLS 安装闭包，不模拟 join 结果。
pub(crate) fn take_manager_start_hook() -> Option<Box<dyn FnOnce() + Send>> {
    MANAGER_START.with(|slot| slot.borrow_mut().take())
}

/// 安装真实 manager 线程的测试启动回调。参数：hook 为单次回调；返回：无。
pub(crate) fn install_manager_start_hook(hook: Box<dyn FnOnce() + Send>) {
    MANAGER_START.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[test]
fn managed_worker_deadline_and_host_finalization_wait_for_distinct_real_threads() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("file"), b"owned worker").unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let (service_tx, service_rx) = channel();
    let (finalize_tx, finalize_rx) = channel();
    let (finalized_tx, finalized_rx) = channel();
    let (manager_reached_tx, manager_reached_rx) = channel();
    let (manager_release_tx, manager_release_rx) = channel();
    let (manager_released_tx, manager_released_rx) = channel();
    // owner 在独立宿主后台线程构造、持有和最终释放，不经过 UI 句柄 Drop。
    let host = thread::spawn(move || {
        MANAGER_START.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                NativeWorkerExitBarrier::install(
                    manager_reached_tx,
                    manager_release_rx,
                    manager_released_tx,
                );
            }));
        });
        let (service, mut owner) = NativeService::new_with_owner(database).unwrap();
        service_tx.send(service).unwrap();
        finalize_rx.recv_timeout(Duration::from_secs(60)).unwrap();
        let result = owner.finalize_owner();
        let _ = finalized_tx.send(result.is_ok());
        result
    });
    let service = service_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    let (worker_reached_tx, worker_reached_rx) = channel();
    let (worker_release_tx, worker_release_rx) = channel();
    let (worker_released_tx, worker_released_rx) = channel();
    let install = Mutex::new(Some((
        worker_reached_tx,
        worker_release_rx,
        worker_released_tx,
    )));
    service.set_scan_progress_hook(Arc::new(move |_| {
        if let Some((reached, release, released)) = install.lock().unwrap().take() {
            NativeWorkerExitBarrier::install(reached, release, released);
        }
    }));
    let handle = service
        .spawn_scan(source.path().to_str().unwrap().to_owned())
        .unwrap();
    let reached = worker_reached_rx.recv_timeout(Duration::from_secs(60));
    let polled = handle.poll_result_json();
    let started = Instant::now();
    let pending = service.drain_coordinators_until(Instant::now() + Duration::from_millis(30));
    let elapsed = started.elapsed();
    let unjoined = !handle.worker.is_joined();
    let worker_release = worker_release_tx.send(());
    let worker_released = worker_released_rx.recv_timeout(Duration::from_secs(10));
    let drained = service.drain_coordinators_until(Instant::now() + Duration::from_secs(10));
    let answer = handle.result_json();
    drop(handle);
    let weak_service = Arc::downgrade(&service);
    drop(service);
    let service_gone = weak_service.upgrade().is_none();
    let finalize_sent = finalize_tx.send(());
    let manager_reached = manager_reached_rx.recv_timeout(Duration::from_secs(10));
    let finalized_early = finalized_rx.recv_timeout(Duration::from_millis(100));
    let manager_release = manager_release_tx.send(());
    let manager_released = manager_released_rx.recv_timeout(Duration::from_secs(10));
    let finalized = host.join();

    eprintln!(
        "managed drain elapsed={elapsed:?}, pending={pending:?}, worker_released={worker_released:?}, manager_released={manager_released:?}"
    );
    assert!(reached.is_ok(), "实际 worker TLS 未到达: {reached:?}");
    assert!(
        elapsed < Duration::from_secs(1),
        "30ms drain 在调用栈同步 join: {elapsed:?}"
    );
    assert!(unjoined, "pending 必须保留尚未 join 的 worker");
    match pending {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "coordinator drain deadline exceeded")
        }
        Ok(()) => panic!("worker TLS 未释放却报告 drain 成功"),
    }
    assert!(worker_release.is_ok());
    assert!(worker_released.unwrap());
    drained.unwrap();
    assert_eq!(Some(answer.clone()), polled);
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["ok"], true, "{answer}");
    assert!(answer["data"]["revision"].is_string());
    assert!(service_gone, "独立 owner 不应强持 NativeService");
    assert!(finalize_sent.is_ok() && manager_reached.is_ok());
    assert!(
        matches!(
            finalized_early,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "manager TLS 未释放却已 finalize: {finalized_early:?}"
    );
    assert!(manager_release.is_ok());
    assert!(manager_released.unwrap());
    finalized.unwrap().unwrap();
}

#[test]
fn host_owner_drop_finalizes_manager_after_service_has_been_dropped() {
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let (service_tx, service_rx) = channel();
    let (drop_tx, drop_rx) = channel();
    let (done_tx, done_rx) = channel();
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let host = thread::spawn(move || {
        MANAGER_START.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                NativeWorkerExitBarrier::install(reached_tx, release_rx, released_tx);
            }));
        });
        let (service, owner) = NativeService::new_with_owner(database).unwrap();
        service_tx.send(service).unwrap();
        drop_rx.recv_timeout(Duration::from_secs(60)).unwrap();
        drop(owner);
        let _ = done_tx.send(());
    });
    let service = service_rx.recv_timeout(Duration::from_secs(60)).unwrap();
    let weak = Arc::downgrade(&service);
    drop(service);
    let service_gone = weak.upgrade().is_none();
    let dropping = drop_tx.send(());
    let reached = reached_rx.recv_timeout(Duration::from_secs(10));
    let early = done_rx.recv_timeout(Duration::from_millis(100));
    let released = release_tx.send(());
    let active_release = released_rx.recv_timeout(Duration::from_secs(10));
    let joined = host.join();
    assert!(service_gone && dropping.is_ok() && reached.is_ok());
    assert!(matches!(
        early,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(released.is_ok() && active_release.unwrap());
    joined.unwrap();
}

#[test]
fn failed_database_construction_does_not_start_a_manager() {
    let data = tempfile::tempdir().unwrap();
    let directory_database = data.path().join("is-a-directory.sqlite");
    std::fs::create_dir(&directory_database).unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    MANAGER_START.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            observed.store(true, Ordering::SeqCst);
        }));
    });
    let result = NativeService::new_with_owner(directory_database.to_str().unwrap().to_owned());
    let failed = match result {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert!(!reason.is_empty());
            true
        }
        Ok((service, mut owner)) => {
            drop(service);
            owner.finalize_owner().unwrap();
            false
        }
    };
    let unconsumed = take_manager_start_hook().is_some();
    assert!(failed, "合法目录不能作为 SQLite 数据库文件打开");
    assert!(!started.load(Ordering::SeqCst));
    assert!(unconsumed, "打开数据库失败前不应创建/启动 manager");
}
