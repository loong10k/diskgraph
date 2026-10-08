//! 注册期限覆盖真实共享锁等待，不把事务末检当作有限等待保证。
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{BusinessError, PrincipalId};
use std::sync::{Arc, mpsc};
use std::time::Duration;

fn assert_registration_deadline(hold_graph: bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: dir.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let principal = PrincipalId::new("scope-deadline-owner").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let graph = hold_graph.then(|| engine.graph().unwrap());
    let control = (!hold_graph).then(|| engine.control().unwrap());
    let later_root = root.clone();
    let later_principal = principal.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker_engine = Arc::clone(&engine);
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(worker_engine.register_scope(&root, &principal, &policy))
            .unwrap();
    });
    started_rx.recv().unwrap();
    let result = done_rx.recv_timeout(Duration::from_millis(5500));
    // 无论红灯或绿灯都先释放真实持锁线程资源，避免断言遗弃执行线程。
    drop(graph);
    drop(control);
    worker.join().unwrap();
    assert!(
        matches!(
            result,
            Ok(Err(EngineError::Business(BusinessError::BudgetExceeded)))
        ),
        "registration waited beyond its original five-second lock budget: {result:?}"
    );
    assert!(engine.control().unwrap().list_scopes().unwrap().is_empty());
    let scope = engine
        .register_scope(
            &later_root,
            &later_principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_eq!(engine.scope(&scope).unwrap().scope_id, scope);
}

#[test]
fn registration_returns_budget_error_while_the_graph_owner_remains_live() {
    assert_registration_deadline(true);
}

#[test]
fn initial_registration_authorization_wait_uses_the_same_deadline() {
    assert_registration_deadline(false);
}
