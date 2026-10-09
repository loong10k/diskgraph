//! 成组 revision 终检的真实 SQL 中断须为业务预算失败，期限不得刷新。

use crate::EngineError;
use crate::relation_request_tests::published_authorization_fixture;
use diskgraph_core::BusinessError;
use std::time::{Duration, Instant};

#[test]
fn grouped_terminal_control_vm_expiry_is_a_business_budget() {
    let (dir, engine, principal, _, revision) = published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    // 隔离夹具在产出 server 身份前执行真实递归聚合，不伪造 SQL 错误或生产时钟。
    db.execute_batch(
        "ALTER TABLE server RENAME TO original_server;
        CREATE VIEW server AS WITH RECURSIVE work(n) AS
        (SELECT 0 UNION ALL SELECT n+1 FROM work WHERE n<500000000)
        SELECT 1 AS id, min(server_id) AS server_id, sum(n) AS cost
        FROM original_server CROSS JOIN work;",
    )
    .unwrap();
    let result = engine.finalize_revisions_read_until(
        &[revision, revision],
        &principal,
        &policy,
        Instant::now() + Duration::from_secs(2),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "{result:?}"
    );
    // 清除夹具慢查询后，同一控制连接必须恢复且不能保留过期 VM 回调。
    db.execute_batch("DROP VIEW server; ALTER TABLE original_server RENAME TO server;")
        .unwrap();
    assert!(
        engine
            .finalize_revisions_read_until(
                &[revision, revision],
                &principal,
                &policy,
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap()
    );
}
