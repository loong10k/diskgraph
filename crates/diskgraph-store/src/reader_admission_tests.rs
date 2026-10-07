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

#[test]
#[ignore = "release diagnostic only; raw reader deliberately lacks production admission hooks"]
fn ownership_reader_configuration_cost_experiment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut configured = Vec::new();
    let mut raw = Vec::new();
    // 同一实际迁移库交替打开，计时涵盖 SQL 与关闭；原始连接不是可交付实现。
    for round in 0..100 {
        for offset in 0..2 {
            let full = (round + offset) % 2 == 0;
            let started = Instant::now();
            let reader = if full {
                SqliteSnapshotStore::open_reader_until(
                    &path,
                    started + Duration::from_secs(5),
                    None,
                )
                .unwrap()
            } else {
                SqliteSnapshotStore {
                    connection: rusqlite::Connection::open_with_flags(
                        &path,
                        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                    )
                    .unwrap(),
                }
            };
            assert!(
                !reader
                    .revision_ownership_matches("missing", "server", "scope")
                    .unwrap()
            );
            drop(reader);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            if full {
                configured.push(elapsed);
            } else {
                raw.push(elapsed);
            }
        }
    }
    configured.sort_by(f64::total_cmp);
    raw.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"experiment":"reader configuration plus missing ownership SQL and close", "os":std::env::consts::OS,
        "production_qualification":false,"samples_per_variant":100,
        "configured":{"p50_ms":configured[49],"p95_ms":configured[94]},
        "raw_unsafe_diagnostic":{"p50_ms":raw[49],"p95_ms":raw[94]}})
    );
}
