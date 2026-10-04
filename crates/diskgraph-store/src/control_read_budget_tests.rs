//! 控制授权窗口必须在成功、错误、到期及 unwind 后还原；来源：Q-08 / 13.6。

use crate::{ControlStore, StoreError};
use std::time::{Duration, Instant};

fn configured_store() -> ControlStore {
    let store = ControlStore::open_in_memory().unwrap();
    store
        .connection
        .busy_timeout(Duration::from_millis(731))
        .unwrap();
    store
}

fn assert_restored(store: &ControlStore) {
    let timeout: i64 = store
        .connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap();
    assert_eq!(timeout, 731);
    // 已过期 handler 如果留在连接上，这次真实 VM 执行会中断。
    assert_eq!(
        store
            .connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn control_deadline_restores_actual_configuration_after_success_and_error() {
    let store = configured_store();
    let success = store
        .with_read_deadline(
            Instant::now() + Duration::from_millis(100),
            |store| -> crate::Result<i64> {
                let temporary: i64 =
                    store
                        .connection
                        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
                assert!(temporary <= 100);
                Ok(store.authorization_generation()? as i64)
            },
        )
        .unwrap();
    assert_eq!(success, 0);
    assert_restored(&store);
    let error = store
        .with_read_deadline(
            Instant::now() + Duration::from_millis(100),
            |_| -> crate::Result<()> { Err(StoreError::IntegerOverflow) },
        )
        .unwrap_err();
    assert!(matches!(error, StoreError::IntegerOverflow));
    assert_restored(&store);
}

#[test]
fn control_deadline_refuses_a_late_success_and_restores_the_connection() {
    let store = configured_store();
    let error = store
        .with_read_deadline(
            Instant::now() + Duration::from_millis(20),
            |_| -> crate::Result<()> {
                std::thread::sleep(Duration::from_millis(40));
                Ok(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, StoreError::BudgetExceeded));
    assert_restored(&store);
}

#[test]
fn control_deadline_restores_the_connection_before_resuming_unwind() {
    let store = configured_store();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: crate::Result<()> =
            store.with_read_deadline(Instant::now() + Duration::from_millis(20), |_| {
                std::thread::sleep(Duration::from_millis(40));
                panic!("real consumer unwind");
            });
    }));
    assert!(result.is_err());
    assert_restored(&store);
}

#[test]
fn control_deadline_interrupts_real_sqlite_execution_and_restores_the_connection() {
    let store = configured_store();
    let error = store.with_read_deadline(Instant::now() + Duration::from_millis(20), |store| -> crate::Result<i64> {
        Ok(store.connection.query_row("WITH RECURSIVE work(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM work WHERE n<1000000000) SELECT sum(n) FROM work", [], |row| row.get(0))?)
    }).unwrap_err();
    assert!(
        error.is_interrupted(),
        "actual SQLite execution was not interrupted: {error}"
    );
    assert_restored(&store);
}
