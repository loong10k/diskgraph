//! PF-06 manager 异常的真实 active/queued 句柄责任，以及旧入口无隐含 manager。
use crate::native_lifecycle::NativeLifecycle;
use crate::native_service_owner_tests::install_manager_start_hook;
use crate::{NativeService, NativeServiceError, NativeServiceOwner};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::sync::{Arc, atomic::AtomicBool, mpsc::channel};
use std::thread;
use std::time::{Duration, Instant};

thread_local! {
    static AFTER_TAKE: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// 测试 manager 已接管 active handle 的精确同步点。
/// 参数：无；返回：无，仅当前 manager 测试线程的单次回调会执行。
pub(crate) fn after_take() {
    if let Some(hook) = AFTER_TAKE.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[test]
fn manager_panic_retains_active_and_queued_handles_until_host_reclaims_them() {
    let (taken_tx, taken_rx) = channel();
    let (panic_tx, panic_rx) = channel();
    let (active_tx, active_rx) = channel();
    let (active_reached_tx, active_reached_rx) = channel();
    let (queued_tx, queued_rx) = channel();
    let (queued_reached_tx, queued_reached_rx) = channel();
    let (fixture_tx, fixture_rx) = channel();
    let (finalize_tx, finalize_rx) = channel();
    let (done_tx, done_rx) = channel();
    let host = thread::spawn(move || {
        install_manager_start_hook(Box::new(move || {
            AFTER_TAKE.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    taken_tx.send(()).unwrap();
                    panic_rx.recv_timeout(Duration::from_secs(60)).unwrap();
                    panic!("real manager failure after taking active handle");
                }))
            });
        }));
        let lifecycle =
            Arc::new(NativeLifecycle::managed(Arc::new(AtomicBool::new(false)), 8).unwrap());
        let mut owner = NativeServiceOwner::start(lifecycle.clone()).unwrap();
        let active = lifecycle
            .register("active".into(), || {
                crate::scan_coordinator::try_spawn_job(move |_, _| {
                    active_reached_tx.send(()).unwrap();
                    active_rx.recv_timeout(Duration::from_secs(60)).unwrap();
                    Ok(json!({"marker":"active"}))
                })
            })
            .unwrap();
        let queued = lifecycle
            .register("queued".into(), || {
                crate::scan_coordinator::try_spawn_job(move |_, _| {
                    queued_reached_tx.send(()).unwrap();
                    queued_rx.recv_timeout(Duration::from_secs(60)).unwrap();
                    Ok(json!({"marker":"queued"}))
                })
            })
            .unwrap();
        fixture_tx.send((lifecycle, active, queued)).unwrap();
        finalize_rx.recv_timeout(Duration::from_secs(60)).unwrap();
        let result = owner.finalize_owner();
        let _ = done_tx.send(result);
    });
    let (lifecycle, active, queued) = fixture_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let reached = taken_rx.recv_timeout(Duration::from_secs(10));
    let active_reached = active_reached_rx.recv_timeout(Duration::from_secs(10));
    let queued_reached = queued_reached_rx.recv_timeout(Duration::from_secs(10));
    let panic_sent = panic_tx.send(());
    lifecycle.close();
    let pending = lifecycle.drain_until(Instant::now() + Duration::from_millis(30));
    let active_pending = !active.worker.is_joined();
    let queued_pending = !queued.worker.is_joined();
    let active_release = active_tx.send(());
    // Active RAII unwind 必须先真正 join；随后 manager failure 唤醒 queued 订阅者，但不写 joined。
    let queued_result = queued.result_json();
    let failure = lifecycle.drain_until(Instant::now() + Duration::from_secs(10));
    let queued_still_pending = !queued.worker.is_joined();
    let finalize_sent = finalize_tx.send(());
    let finalize_pending = done_rx.recv_timeout(Duration::from_millis(100));
    let queued_release = queued_tx.send(());
    let finalized = done_rx.recv_timeout(Duration::from_secs(10));
    let joined = host.join();

    assert!(reached.is_ok() && active_reached.is_ok() && queued_reached.is_ok());
    assert!(panic_sent.is_ok() && active_release.is_ok() && queued_release.is_ok());
    assert!(active_pending && queued_pending && queued_still_pending);
    assert_reason(pending, "coordinator drain deadline exceeded");
    assert_reason(failure, "coordinator join manager failed");
    let result: Value = serde_json::from_str(&queued_result).unwrap();
    assert_eq!(result["ok"], false);
    assert!(queued_result.contains("coordinator join manager failed"));
    assert!(finalize_sent.is_ok());
    assert!(matches!(
        finalize_pending,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    assert_reason(finalized.unwrap(), "coordinator join manager failed");
    joined.unwrap();
    assert!(active.worker.is_joined() && queued.worker.is_joined());
}

#[test]
fn legacy_drain_is_explicitly_unsupported_and_poll_only_scans_do_not_exhaust_records() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), b"legacy compatibility").unwrap();
    let service = NativeService::new(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let limit = diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
    for index in 0..(2 * limit + 1) {
        let handle = service
            .spawn_scan(root.path().to_str().unwrap().to_owned())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let result = loop {
            if let Some(result) = handle.poll_result_json() {
                break result;
            }
            assert!(
                Instant::now() < deadline,
                "legacy scan {index} did not finish"
            );
            thread::sleep(Duration::from_millis(5));
        };
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert!(result["data"]["revision"].is_string());
        drop(handle);
    }
    assert_reason(
        service.drain_coordinators_until(Instant::now() + Duration::from_secs(1)),
        "unsupported: legacy session has no managed coordinator owner",
    );
}

#[test]
fn managed_worker_cannot_wait_for_its_own_record_after_manager_takes_handle() {
    let (taken_tx, taken_rx) = channel();
    install_manager_start_hook(Box::new(move || {
        AFTER_TAKE.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                taken_tx.send(()).unwrap();
            }))
        });
    }));
    let lifecycle =
        Arc::new(NativeLifecycle::managed(Arc::new(AtomicBool::new(false)), 8).unwrap());
    let mut owner = NativeServiceOwner::start(lifecycle.clone()).unwrap();
    let worker_lifecycle = lifecycle.clone();
    let (check_tx, check_rx) = channel();
    let (answer_tx, answer_rx) = channel();
    let handle = lifecycle
        .register("self-wait".into(), || {
            crate::scan_coordinator::try_spawn_job(move |_, _| {
                check_rx.recv_timeout(Duration::from_secs(60)).unwrap();
                let result = worker_lifecycle.drain_until(Instant::now() + Duration::from_secs(1));
                answer_tx.send(result).unwrap();
                Ok(json!({"marker":"self-wait-refused"}))
            })
        })
        .unwrap();
    let taken = taken_rx.recv_timeout(Duration::from_secs(10));
    let checked = check_tx.send(());
    let answer = answer_rx.recv_timeout(Duration::from_secs(10));
    let result = handle.result_json();
    owner.finalize_owner().unwrap();
    assert!(taken.is_ok() && checked.is_ok());
    assert_reason(answer.unwrap(), "coordinator cannot join its own thread");
    assert!(handle.worker.is_joined());
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["marker"], "self-wait-refused");
}

fn assert_reason(result: Result<(), NativeServiceError>, expected: &str) {
    match result {
        Err(NativeServiceError::Unavailable { reason }) => assert_eq!(reason, expected),
        Ok(()) => panic!("expected {expected}, not a completed drain"),
    }
}
