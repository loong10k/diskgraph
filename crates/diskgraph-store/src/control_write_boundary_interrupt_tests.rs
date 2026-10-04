//! D44 事务边界中断分类；来源：真实 SQLite prepare/VM 与原请求期限回归。
use crate::StoreError;
use crate::control_write_deadline::ControlWriteDeadline;
use crate::process_job_enqueue_fixture::ProcessEnqueueFixture;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn wait_past(deadline: Instant) {
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining + Duration::from_millis(20));
    }
}

#[test]
fn control_write_begin_interrupted_after_prepare_exhausts_the_original_deadline() {
    // 同 literal 的 raw 正控证明真实 VM 中断；随后 guard 入口必须保持旧预算分类。
    for raw_companion in [true, false] {
        let fixture = ProcessEnqueueFixture::new();
        fixture.set_busy_timeout(731);
        let before = fixture.persisted_state();
        let reached = Arc::new(AtomicUsize::new(0));
        let callback_started_live = Arc::new(AtomicBool::new(false));
        let callback_expired = Arc::new(AtomicBool::new(false));
        let count = Arc::clone(&reached);
        let live = Arc::clone(&callback_started_live);
        let expired = Arc::clone(&callback_expired);
        // 一次 2s 准备窗不是响应性能阈值；只由真实 prepare 回调消耗，不续期。
        let deadline = Instant::now() + Duration::from_secs(2);
        let window = ControlWriteDeadline::new(
            &fixture.store.connection,
            deadline,
            fixture.authority.expires_at_unix_seconds(),
        )
        .unwrap();
        fixture
            .store
            .connection
            .authorizer(Some(move |context: AuthContext<'_>| {
                if matches!(
                    context.action,
                    AuthAction::Transaction {
                        operation: TransactionOperation::Begin
                    }
                ) {
                    count.fetch_add(1, Ordering::SeqCst);
                    live.store(Instant::now() < deadline, Ordering::SeqCst);
                    // 回调仅等待并 Allow，无 SQL 重入、无 panic、无权限替身。
                    wait_past(deadline);
                    expired.store(Instant::now() >= deadline, Ordering::SeqCst);
                }
                Authorization::Allow
            }))
            .unwrap();
        let before_begin_live = window.check().is_ok();
        let result = if raw_companion {
            fixture
                .store
                .connection
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(StoreError::from)
        } else {
            window.begin()
        };
        let result = window.finish(result);
        fixture
            .store
            .connection
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        let persisted = fixture.persisted_state();
        fixture.assert_connection_restored(731);
        eprintln!(
            "D44_BEGIN_INTERRUPT {}",
            serde_json::json!({
                "raw_companion": raw_companion,
                "before_begin_live": before_begin_live,
                "callback_count": reached.load(Ordering::SeqCst),
                "callback_started_live": callback_started_live.load(Ordering::SeqCst),
                "callback_expired": callback_expired.load(Ordering::SeqCst),
                "result": format!("{result:?}")
            })
        );
        assert!(before_begin_live, "guard was already expired before BEGIN");
        assert_eq!(reached.load(Ordering::SeqCst), 1);
        assert!(callback_started_live.load(Ordering::SeqCst));
        assert!(callback_expired.load(Ordering::SeqCst));
        assert_eq!(persisted, before);
        if raw_companion {
            assert!(
                matches!(&result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _))) if code.code == rusqlite::ErrorCode::OperationInterrupted),
                "raw SQLite companion never interrupted: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(StoreError::BudgetExceeded)),
                "original deadline interruption changed category: {result:?}"
            );
        }
    }
}

#[test]
fn control_write_live_external_vm_interruption_keeps_the_real_sqlite_error() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let deadline = Instant::now() + Duration::from_secs(2);
    let window = ControlWriteDeadline::new(
        &fixture.store.connection,
        deadline,
        fixture.authority.expires_at_unix_seconds(),
    )
    .unwrap();
    let interrupted = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&interrupted);
    fixture
        .store
        .connection
        .progress_handler(
            1,
            Some(move || {
                // 外部 VM 停止请求真实触发 SQLITE_INTERRUPT，不是到期门禁或 SQL 重入。
                mark.store(true, Ordering::SeqCst);
                true
            }),
        )
        .unwrap();
    let before_begin_live = window.check().is_ok();
    let result = window.begin();
    let still_live = Instant::now() < deadline;
    let result = window.finish(result);
    let persisted = fixture.persisted_state();
    wait_past(deadline);
    fixture.assert_connection_restored(731);
    assert!(before_begin_live && still_live);
    assert!(interrupted.load(Ordering::SeqCst));
    assert!(
        matches!(&result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _))) if code.code == rusqlite::ErrorCode::OperationInterrupted),
        "live SQLite interruption was masked: {result:?}"
    );
    assert_eq!(persisted, before);
}
