//! PF-06 真实线程与有界登记回归；不把这些协调线程当作 pinned walker。
use crate::native_lifecycle::NativeLifecycle;
use crate::native_worker_exit_barrier::NativeWorkerExitBarrier;
use crate::{NativeService, NativeServiceError, NativeServiceOwner};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::channel,
};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn tls_pending_record_survives_deadline_then_concurrent_waiters_join_it() {
    let closed = Arc::new(AtomicBool::new(false));
    let lifecycle = Arc::new(NativeLifecycle::managed(closed, 8).unwrap());
    let mut owner = NativeServiceOwner::start(lifecycle.clone()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let (reached_tx, reached_rx) = channel();
    let (release_tx, release_rx) = channel();
    let (released_tx, released_rx) = channel();
    let handle = lifecycle
        .register(root.path().to_path_buf(), || {
            crate::scan_coordinator::try_spawn_job(move |_, _| {
                NativeWorkerExitBarrier::install(reached_tx, release_rx, released_tx);
                Ok(json!({"marker":"retained_tls"}))
            })
        })
        .unwrap();
    let reached = reached_rx.recv_timeout(Duration::from_secs(10));
    let polled = handle.poll_result_json();
    lifecycle.close();
    let drain_started = Instant::now();
    let pending = lifecycle.drain_until(Instant::now() + Duration::from_millis(30));
    let drain_elapsed = drain_started.elapsed();
    let worker = handle.worker.clone();
    let still_unjoined = !worker.is_joined();
    let subscriber = Arc::downgrade(&handle);
    let result_reader = thread::spawn(move || handle.result_json());
    let drains: Vec<_> = (0..2)
        .map(|_| {
            let lifecycle = lifecycle.clone();
            thread::spawn(move || lifecycle.drain_until(Instant::now() + Duration::from_secs(10)))
        })
        .collect();
    let release_sent = release_tx.send(());
    let released = released_rx.recv_timeout(Duration::from_secs(10));
    let result = result_reader.join();
    let drained: Vec<_> = drains.into_iter().map(thread::JoinHandle::join).collect();

    eprintln!(
        "tls_drain elapsed={drain_elapsed:?} release_sent={release_sent:?} released={released:?} pending={pending:?}"
    );
    assert!(reached.is_ok());
    assert!(
        drain_elapsed < Duration::from_secs(1),
        "30ms drain 不得主动阻塞 join 至 TLS 救援释放: {drain_elapsed:?}"
    );
    assert!(release_sent.is_ok());
    assert!(released.unwrap());
    match pending {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "coordinator drain deadline exceeded")
        }
        Ok(()) => panic!("TLS 尚未退出却声称 drain 完成"),
    }
    assert!(still_unjoined, "deadline 不能丢弃尚未 join 的记录");
    for answer in drained {
        answer.unwrap().unwrap();
    }
    assert_eq!(Some(result.unwrap()), polled);
    assert!(worker.is_joined());
    assert!(subscriber.upgrade().is_none(), "登记表不得强持宿主订阅者");
    owner.finalize_owner().unwrap();
}

#[test]
fn retained_limit_allows_same_root_and_last_subscriber_cancels() {
    let limit = diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
    let lifecycle =
        Arc::new(NativeLifecycle::managed(Arc::new(AtomicBool::new(false)), limit).unwrap());
    let mut owner = NativeServiceOwner::start(lifecycle.clone()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let mut handles = Vec::new();
    let mut releases = Vec::new();
    for index in 0..limit {
        let (release_tx, release_rx) = channel();
        releases.push(release_tx);
        handles.push(
            lifecycle
                .register(root.path().join(index.to_string()), || {
                    crate::scan_coordinator::try_spawn_job(move |_, _| {
                        release_rx.recv_timeout(Duration::from_secs(60)).unwrap();
                        Ok(json!({"done":true}))
                    })
                })
                .unwrap(),
        );
    }
    let shared = lifecycle
        .register(root.path().join("0"), || panic!("同根合并不得创建线程"))
        .unwrap();
    let coalesced = Arc::ptr_eq(&shared, &handles[0]);
    let cancel = shared.cancel.clone();
    drop(shared);
    let first_drop_kept_alive = !cancel.load(Ordering::SeqCst);
    let last_subscriber = handles.remove(0);
    let weak = Arc::downgrade(&last_subscriber);
    drop(last_subscriber);
    let last_drop_cancelled = cancel.load(Ordering::SeqCst);
    let denied = lifecycle.register(root.path().join("overflow"), || {
        panic!("额度满时不得启动线程")
    });
    lifecycle.close();
    for release in releases {
        release.send(()).unwrap();
    }
    let drained = lifecycle.drain_until(Instant::now() + Duration::from_secs(10));
    for handle in handles {
        handle.worker.join().unwrap();
    }

    assert!(coalesced && first_drop_kept_alive && last_drop_cancelled);
    assert!(weak.upgrade().is_none());
    match denied {
        Err(NativeServiceError::Unavailable { reason }) => {
            assert_eq!(reason, "resource_exhausted: native coordinators")
        }
        Ok(_) => panic!("活动协调线程超过保留额度"),
    }
    drained.unwrap();
    owner.finalize_owner().unwrap();
}

#[test]
fn poll_only_host_can_complete_more_scans_than_the_retained_limit() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("ordinary"), b"native lifecycle").unwrap();
    let (service, mut owner) = NativeService::new_with_owner(
        data.path()
            .join("graph.sqlite")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .unwrap();
    let limit = diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
    for index in 0..(limit * 2 + 1) {
        let handle = service
            .spawn_scan(root.path().to_str().unwrap().to_owned())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let answer = loop {
            if let Some(answer) = handle.poll_result_json() {
                break answer;
            }
            assert!(Instant::now() < deadline, "第 {index} 次实际扫描未完成");
            thread::sleep(Duration::from_millis(5));
        };
        let answer: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(answer["ok"], true, "第 {index} 次扫描: {answer}");
        assert!(answer["data"]["revision"].is_string(), "{answer}");
        // 刻意不调用 result_json；下一次准入必须回收真正结束的 coordinator。
        drop(handle);
    }
    service
        .drain_coordinators_until(Instant::now() + Duration::from_secs(10))
        .unwrap();
    owner.finalize_owner().unwrap();
}
