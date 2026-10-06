//! D42 原入口控制锁窗口验收；来源：同 Engine Mutex 与实际 revision owner 窄读，不用授权睡眠。
#[cfg(not(target_os = "linux"))]
use crate::Engine;
#[cfg(target_os = "linux")]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{BusinessError, JobRequestAuthority, PrincipalId};
use std::cell::RefCell;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

thread_local! {
    static AFTER_OWNER: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// 参数：无；返回：只消费当前请求线程的一次实际 owner-read 后同步点。
pub(crate) fn after_owner_read() {
    if let Some(hook) = AFTER_OWNER.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[test]
fn process_entry_control_contention_is_bounded_across_the_whole_original_request() {
    let source = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("target"), b"fixture").unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: data.path().to_owned(),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let actor = PrincipalId::new("process-entry-lock").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(source.path(), &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let auth = engine.policy_authorizer().unwrap();
    let scan = engine.index_scope(&scope, &actor, &auth).unwrap();
    engine.run_job(&scan.job_id, "fixture-scan").unwrap();
    let revision = engine
        .revision_for_job(&scan.job_id, &actor, &auth)
        .unwrap();
    let node = engine
        .revision_node_at(&revision, Path::new("target"))
        .unwrap()
        .unwrap();
    let authority = JobRequestAuthority::trusted_local(actor, "trusted-engine").unwrap();
    let (reached_tx, reached_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let request = Arc::clone(&engine);
    let worker = std::thread::spawn(move || {
        AFTER_OWNER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                reached_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }))
        });
        let started = Instant::now();
        let result = request
            .process_evidence_scope_with_authority(&scope, &revision, node.id, &authority, &auth);
        let elapsed = started.elapsed();
        let consumed = AFTER_OWNER.with(|slot| slot.borrow().is_none());
        done_tx.send((result, elapsed, consumed)).unwrap();
    });
    let reached = reached_rx.recv_timeout(Duration::from_secs(10));
    if reached.is_err() {
        drop(resume_tx);
        let _ = worker.join();
        panic!("fixture never reached real owner-read control window: {reached:?}");
    }
    let control = engine.control_store().unwrap();
    resume_tx.send(()).unwrap();
    // 真实同一把控制锁跨越原1秒期限；受限实现应立即拒绝，不能等释放后才发现迟到。
    let completed_while_locked = done_rx.recv_timeout(Duration::from_millis(1100));
    drop(control);
    let delivered_before_release = completed_while_locked.is_ok();
    let (result, elapsed, consumed) = completed_while_locked
        .unwrap_or_else(|_| done_rx.recv_timeout(Duration::from_secs(10)).unwrap());
    worker.join().unwrap();
    assert!(
        consumed,
        "synchronization hook must be consumed by actual entry"
    );
    assert!(
        delivered_before_release,
        "entry waited on blocking control Mutex for {elapsed:?}: {result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "original entry deadline must not be refreshed: {elapsed:?}"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
                | Err(EngineError::Store(
                    diskgraph_store::StoreError::BudgetExceeded
                ))
        ),
        "actual refusal: {result:?}"
    );
    assert!(
        engine.queued_jobs().unwrap().is_empty(),
        "failed preparation must not enqueue work"
    );
}
