//! 策略能力捕获必须保留原期限的预算错误，不泄露通用 SQLite 中断。

use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{BusinessError, PrincipalId};
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, Engine, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    engine
        .control_store()
        .unwrap()
        .publish_policy_version(1)
        .unwrap();
    let db = rusqlite::Connection::open(data.join("diskgraph-control.sqlite")).unwrap();
    (dir, engine, db)
}

fn interrupted_capture(principal_only: bool) {
    let (_dir, engine, db) = fixture();
    // 隔离数据库中的真实递归 VM 在产出首行前做工作，原 progress guard 必须中断。
    // 不替换时钟、授权决定或 SQL 错误；递归聚合以常量空间执行。
    db.execute_batch(
        "ALTER TABLE grants RENAME TO original_grants;
        CREATE VIEW grants AS WITH RECURSIVE work(n) AS
        (SELECT 0 UNION ALL SELECT n+1 FROM work WHERE n<500000000)
        SELECT 'alice' AS principal_id, 'metadata:read' AS permission,
        'scope' AS scope_id, sum(n)*0+1 AS policy_version FROM work HAVING sum(n)>0;",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let result = if principal_only {
        engine.policy_authorizer_for_principal_until(&PrincipalId::new("alice").unwrap(), deadline)
    } else {
        engine.policy_authorizer_until(deadline)
    };
    let Err(error) = result else {
        panic!("the actual recursive query must expire");
    };
    assert!(
        matches!(error, EngineError::Business(BusinessError::BudgetExceeded)),
        "{error}"
    );
    db.execute_batch("DROP VIEW grants; ALTER TABLE original_grants RENAME TO grants;")
        .unwrap();
    assert!(
        engine
            .policy_authorizer_until(Instant::now() + Duration::from_secs(1))
            .is_ok()
    );
}

#[test]
fn principal_policy_capture_maps_actual_vm_deadline_to_budget() {
    interrupted_capture(true);
}

#[test]
fn full_policy_capture_maps_actual_vm_deadline_to_budget() {
    interrupted_capture(false);
}

#[test]
fn policy_capture_preserves_nonbudget_database_failure() {
    let (_dir, engine, db) = fixture();
    db.execute_batch("ALTER TABLE grants RENAME TO original_grants;")
        .unwrap();
    let result = engine.policy_authorizer_for_principal_until(
        &PrincipalId::new("alice").unwrap(),
        Instant::now() + Duration::from_secs(1),
    );
    let Err(error) = result else {
        panic!("missing real grant table must fail");
    };
    assert!(matches!(error, EngineError::Store(_)), "{error}");
}
