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
    static AFTER_OPEN: std::cell::RefCell<Option<ConfigurationHook>> = const { std::cell::RefCell::new(None) };
    static AFTER_TEMP_STORE: std::cell::RefCell<Option<ConfigurationHook>> = const { std::cell::RefCell::new(None) };
}

/// 仅在真实连接准备边界观察SQL，不进入生产构建或替换SQLite执行。
type ConfigurationHook = Box<dyn FnOnce(&rusqlite::Connection)>;

/// 参数：刚打开的原连接；返回：无，仅调用本线程一次性测试观察。
pub(crate) fn after_open(connection: &rusqlite::Connection) {
    if let Some(hook) = AFTER_OPEN.with(|slot| slot.borrow_mut().take()) {
        hook(connection);
    }
}

/// 参数：已完成temp_store的原连接；返回：无，不替换或重试配置SQL。
pub(crate) fn after_temp_store(connection: &rusqlite::Connection) {
    if let Some(hook) = AFTER_TEMP_STORE.with(|slot| slot.borrow_mut().take()) {
        hook(connection);
    }
}

fn count_configuration_sql(
    connection: &rusqlite::Connection,
    calls: Arc<std::sync::atomic::AtomicUsize>,
) {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Pragma {
                    pragma_name: "temp_store" | "cache_size",
                    ..
                }
            ) {
                calls.fetch_add(1, Ordering::Relaxed);
            }
            Authorization::Allow
        }))
        .unwrap();
}

#[test]
fn expiry_after_open_starts_no_configuration_sql() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let deadline = Instant::now() + Duration::from_secs(1);
    AFTER_OPEN.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |connection| {
            count_configuration_sql(connection, observed);
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
        }));
    });
    assert!(matches!(
        SqliteSnapshotStore::open_reader_until(&path, deadline, None),
        Err(StoreError::BudgetExceeded)
    ));
    assert!(
        AFTER_OPEN.with(|slot| slot.borrow().is_none()),
        "actual connection-open boundary required"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "expired preparation began real configuration SQL"
    );
}

#[test]
fn cancellation_after_temp_store_starts_no_cache_configuration_sql() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    AFTER_TEMP_STORE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |connection| {
            count_configuration_sql(connection, observed);
            flag.store(true, Ordering::Relaxed);
        }));
    });
    let error = SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        Some(cancel),
    )
    .err()
    .expect("cancelled preparation must fail");
    assert!(error.is_interrupted(), "{error}");
    assert!(
        AFTER_TEMP_STORE.with(|slot| slot.borrow().is_none()),
        "actual first configuration SQL boundary required"
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "cancelled preparation began real cache_size SQL"
    );
}

#[test]
fn cache_configuration_busy_wait_uses_remaining_original_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let remaining = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observed = Arc::clone(&remaining);
    AFTER_TEMP_STORE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |_| {
            // 只消耗足以区分两次配置的预算，避免正控主动耗尽绝大部分原期限。
            std::thread::sleep(Duration::from_millis(20));
            observed.store(
                u64::try_from(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis(),
                )
                .unwrap(),
                Ordering::Relaxed,
            );
        }));
    });
    let reader = SqliteSnapshotStore::open_reader_until(&path, deadline, None).unwrap();
    let busy: i64 = reader
        .connection
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .unwrap();
    let bound = remaining.load(Ordering::Relaxed);
    assert!(bound > 0 && bound < 1000, "real remaining budget: {bound}");
    assert!(
        busy >= 0 && u64::try_from(busy).unwrap() <= bound,
        "busy wait {busy}ms exceeds remaining original {bound}ms"
    );
    assert!(AFTER_TEMP_STORE.with(|slot| slot.borrow().is_none()));
}

#[test]
fn live_preparation_still_executes_both_required_configuration_statements() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    AFTER_OPEN.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |connection| {
            count_configuration_sql(connection, observed)
        }))
    });
    let reader = SqliteSnapshotStore::open_reader_until(
        &path,
        Instant::now() + Duration::from_secs(5),
        None,
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(reader.connection.is_readonly("main").unwrap());
    assert_eq!(
        reader
            .connection
            .pragma_query_value(None, "temp_store", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        reader
            .connection
            .pragma_query_value(None, "cache_size", |row| row.get::<_, i64>(0))
            .unwrap(),
        -8192
    );
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

#[test]
#[ignore = "release phase diagnostic only; not an optimized production reader"]
fn ownership_reader_configuration_phase_costs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.sqlite");
    let _writer = SqliteSnapshotStore::open(&path).unwrap();
    let mut phases: std::collections::BTreeMap<&str, Vec<f64>> = std::collections::BTreeMap::new();
    // 同一隔离真实迁移库、原配置顺序；阶段记录开销不进入下一阶段计时。
    for _ in 0..100 {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut previous = Instant::now();
        let mut mark = |label| {
            let elapsed = previous.elapsed().as_secs_f64() * 1000.0;
            phases.entry(label).or_default().push(elapsed);
            previous = Instant::now();
        };
        let connection = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        mark("open");
        connection
            .busy_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1)),
            )
            .unwrap();
        mark("busy_timeout");
        connection
            .pragma_update(None, "temp_store", "FILE")
            .unwrap();
        mark("temp_store");
        connection.pragma_update(None, "cache_size", -8192).unwrap();
        mark("cache_size");
        connection
            .progress_handler(1000, Some(move || Instant::now() >= deadline))
            .unwrap();
        mark("progress_handler");
        let reader = SqliteSnapshotStore { connection };
        assert!(
            !reader
                .revision_ownership_matches("missing", "server", "scope")
                .unwrap()
        );
        mark("ownership_sql");
        drop(reader);
        mark("close");
    }
    let output: std::collections::BTreeMap<_, _> = phases
        .into_iter()
        .map(|(label, mut samples)| {
            samples.sort_by(f64::total_cmp);
            (
                label,
                serde_json::json!({"samples":100,"p50_ms":samples[49],"p95_ms":samples[94]}),
            )
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({"experiment":"configured reader isolated phase cost", "os":std::env::consts::OS,
        "profile":if cfg!(debug_assertions) {"debug"} else {"release"}, "native_scan_qualification":false,
        "configuration_values_and_order_match_production":true,"admission_checks_profiled":false,"phases":output})
    );
}
