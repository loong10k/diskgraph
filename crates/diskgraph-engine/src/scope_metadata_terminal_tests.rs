//! 隔离 scope 元数据终检的原控制锁竞争，不执行扫描或安装服务。
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{BusinessError, PrincipalId};
use std::cell::RefCell;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

thread_local! {
    static BEFORE_WITNESS: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

/// 参数：无；返回：无。仅测试消费一次性同步，不改变生产授权或期限。
pub(super) fn before_witness_observation() {
    BEFORE_WITNESS.with(|hook| {
        if let Some(callback) = hook.borrow_mut().take() {
            callback();
        }
    });
}

#[test]
fn terminal_witness_lock_uses_database_window_instead_of_remaining_request_budget() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("scope");
    std::fs::create_dir(&root).unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let principal = PrincipalId::new("metadata-window-reader").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let scope = engine.register_scope(&root, &principal, &policy).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder_engine = Arc::clone(&engine);
    let (holder_tx, holder_rx) = mpsc::channel();
    // 真实末段能力和实时 SQL 完成之后，另一请求持有同一原控制锁。
    BEFORE_WITNESS.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let holder = std::thread::spawn(move || {
                let guard = holder_engine.control_store().unwrap();
                held_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                drop(guard);
            });
            holder_tx.send(holder).unwrap();
            held_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }));
    });
    let started = Instant::now();
    let result = engine.with_scope_metadata_until(
        &scope,
        &principal,
        &policy,
        started + Duration::from_secs(2),
        |_| Ok("encoded-private-metadata"),
    );
    let elapsed = started.elapsed();
    // 先回收原持锁线程，再断言；失败不能遗失原线程或等待责任。
    release_tx.send(()).unwrap();
    holder_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .join()
        .unwrap();
    eprintln!(
        "DG_SCOPE_TERMINAL_WITNESS_ELAPSED_US={}",
        elapsed.as_micros()
    );
    assert!(BEFORE_WITNESS.with(|hook| hook.borrow().is_none()));
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "{result:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "terminal witness consumed original request budget: {elapsed:?}"
    );
}
