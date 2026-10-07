//! 独立读取连接的原始期限与取消准入回归；来源：OpenSpec Q-02。
use crate::{SqliteSnapshotStore, StoreError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[test]
fn expired_reader_refuses_before_database_path_access() {
    let dir = tempfile::tempdir().unwrap();
    let result = SqliteSnapshotStore::open_reader_until(
        &dir.path().join("missing.sqlite"),
        Instant::now() - Duration::from_secs(1),
        None,
    );
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
}

#[test]
fn expired_reader_does_not_return_a_connection_for_short_queries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    assert!(matches!(
        SqliteSnapshotStore::open_reader_until(
            &path,
            Instant::now() - Duration::from_secs(1),
            None
        ),
        Err(StoreError::BudgetExceeded)
    ));
}

#[test]
fn cancelled_reader_refuses_before_database_path_access() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(true));
    match SqliteSnapshotStore::open_reader_until(
        &dir.path().join("missing.sqlite"),
        Instant::now() + Duration::from_secs(5),
        Some(cancel),
    ) {
        Err(error) => assert!(
            error.is_interrupted(),
            "unexpected cancellation error: {error}"
        ),
        Ok(_) => panic!("cancelled admission returned a usable reader"),
    }
}

#[test]
fn admitted_reader_keeps_runtime_cancellation_hook() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let reader = SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        Some(cancel.clone()),
    )
    .unwrap();
    assert_eq!(
        reader
            .connection
            .query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    cancel.store(true, Ordering::Relaxed);
    let error=reader.connection.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT sum(x) FROM n",[],|r|r.get::<_,i64>(0)).unwrap_err();
    assert!(StoreError::from(error).is_interrupted());
}

thread_local! {
    static AFTER_PREPARE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// 测试专用准备边界回调，生产构建不包含回调或可注入入口。
pub(crate) fn after_prepare() {
    let hook = AFTER_PREPARE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[test]
fn cancellation_during_reader_preparation_refuses_the_prepared_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    AFTER_PREPARE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || flag.store(true, Ordering::Relaxed)))
    });
    match SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        Some(cancel.clone()),
    ) {
        Err(error) => assert!(
            error.is_interrupted(),
            "unexpected prepared cancellation: {error}"
        ),
        Ok(_) => panic!("preparation cancellation returned a reader"),
    }
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn expiry_during_reader_preparation_refuses_the_prepared_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let flag = reached.clone();
    let deadline = Instant::now() + Duration::from_secs(1);
    AFTER_PREPARE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            flag.store(true, Ordering::Relaxed);
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
        }))
    });
    assert!(matches!(
        SqliteSnapshotStore::open_reader_until(&path, deadline, None),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(
        reached.load(Ordering::Relaxed),
        "fixture did not reach the preparation boundary"
    );
}

#[test]
fn expired_and_cancelled_reader_keeps_deadline_error_precedence() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        SqliteSnapshotStore::open_reader_until(
            &dir.path().join("missing.sqlite"),
            Instant::now() - Duration::from_secs(1),
            Some(Arc::new(AtomicBool::new(true)))
        ),
        Err(StoreError::BudgetExceeded)
    ));
}
