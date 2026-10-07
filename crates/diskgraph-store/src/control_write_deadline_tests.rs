//! D44 真实提交竞争、VM 中断与 Rust unwind 清理，不以事务内部状态断言替代持久结果。
use crate::StoreError;
use crate::control_write_deadline::ControlWriteDeadline;
use crate::process_job_enqueue_fixture::ProcessEnqueueFixture;
use rusqlite::{Connection, Statement};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[test]
fn process_enqueue_commit_read_lock_keeps_the_original_deadline_and_rolls_back() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    fixture
        .store
        .connection
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    let before = fixture.persisted_state();
    let path = fixture.database_path();
    let held = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&held);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN").unwrap();
        let count: i64 = connection
            .query_row("SELECT count(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        held.store(true, Ordering::SeqCst);
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_millis(1200));
        held.store(false, Ordering::SeqCst);
        connection.execute_batch("ROLLBACK").unwrap();
    });
    if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
        let _ = release_tx.send(());
        let joined = reader.join();
        panic!("reader did not acquire a real shared lock: {error:?}; {joined:?}");
    }
    let inserted = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&inserted);
    fixture
        .store
        .connection
        .update_hook(Some(
            move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if action == rusqlite::hooks::Action::SQLITE_INSERT
                    && table == "process_evidence_job_inputs"
                {
                    mark.store(true, Ordering::SeqCst);
                }
            },
        ))
        .unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_millis(150);
    let live_at_call = Instant::now() < deadline;
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    let elapsed = started.elapsed();
    let returned_while_held = observed.load(Ordering::SeqCst);
    let _ = release_tx.send(());
    reader.join().unwrap();
    fixture
        .store
        .connection
        .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
        .unwrap();
    assert!(live_at_call);
    assert!(
        inserted.load(Ordering::SeqCst),
        "never reached the actual final input INSERT: result={result:?}, elapsed={elapsed:?}, returned_while_held={returned_while_held}"
    );
    assert!(
        matches!(result, Err(StoreError::BudgetExceeded)),
        "actual={result:?}"
    );
    assert!(
        returned_while_held,
        "COMMIT waited for reader release: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_millis(500));
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}

const PARTIAL_INSERT: &str = "INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,owner,principal) VALUES('guard-partial',?1,'process_evidence','queued',1,1,'',?2)";
const PARTIAL_EXISTS: &str = "SELECT count(*) FROM jobs WHERE job_id='guard-partial'";
const VM_ROWS: &str = "WITH RECURSIVE work(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM work WHERE n<1000000000) SELECT n FROM work";
const UNWIND_SENTINEL: &str = "D44 actual Rust consumer unwind after witnessed partial write";

// 语句在原时钟前准备；这里证明真实写入仍处于同一事务，而非进入阶段前的预算拒绝。
fn write_partial_job(
    fixture: &ProcessEnqueueFixture,
    window: &ControlWriteDeadline<'_>,
    insert: &mut Statement<'_>,
    exists: &mut Statement<'_>,
) {
    let affected = window
        .statement(|| {
            Ok(insert.execute(rusqlite::params![
                fixture.input.scope_id().as_str(),
                fixture.authority.principal().as_str()
            ])?)
        })
        .unwrap();
    assert_eq!(affected, 1, "actual partial INSERT affected no row");
    assert!(!fixture.store.connection.is_autocommit());
    let rows: i64 = window
        .statement(|| Ok(exists.query_row([], |row| row.get(0))?))
        .unwrap();
    assert_eq!(
        rows, 1,
        "partial job is not visible in the held transaction"
    );
}

fn wait_past_original_deadline(deadline: Instant) {
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining + Duration::from_millis(20));
    }
}

#[test]
fn control_write_real_vm_interrupt_clears_progress_before_partial_transaction_rollback() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let mut insert = fixture.store.connection.prepare(PARTIAL_INSERT).unwrap();
    let mut exists = fixture.store.connection.prepare(PARTIAL_EXISTS).unwrap();
    let mut query = fixture.store.connection.prepare(VM_ROWS).unwrap();
    assert!(fixture.store.connection.is_autocommit());
    // 2s 是一次固定的阶段准备窗口，不是性能阈值；SQL 编译不占该窗口，也不刷新它。
    let started = Instant::now();
    let deadline = started + Duration::from_secs(2);
    let window = ControlWriteDeadline::new(
        &fixture.store.connection,
        deadline,
        fixture.authority.expires_at_unix_seconds(),
    )
    .unwrap();
    window.begin().unwrap();
    write_partial_job(&fixture, &window, &mut insert, &mut exists);
    let interrupted = AtomicBool::new(false);
    let query_started_live = Cell::new(false);
    let observed_rows = Cell::new(0_u64);
    let result = window.statement(|| -> crate::Result<u64> {
        query_started_live.set(Instant::now() < deadline);
        let mut rows = query.query([])?;
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let actual: i64 = row.get(0)?;
                    if observed_rows.get() == 0 {
                        assert_eq!(actual, 0, "recursive query's first row changed");
                    }
                    observed_rows.set(observed_rows.get() + 1);
                }
                Ok(None) => return Ok(observed_rows.get()),
                Err(error) => {
                    // 普通 Rust 调用栈观察真实 SQLite 错误，首行见证排除预检超时冒充 VM 中断。
                    if matches!(&error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::OperationInterrupted) {
                        interrupted.store(true, Ordering::SeqCst);
                    }
                    return Err(error.into());
                }
            }
        }
    });
    let result = window.finish(result);
    eprintln!(
        "D44_VM_STAGE {}",
        serde_json::json!({
            "original_window_ms": 2000,
            "elapsed_us": started.elapsed().as_micros(),
            "query_started_live": query_started_live.get(),
            "observed_rows": observed_rows.get(),
            "raw_operation_interrupted": interrupted.load(Ordering::SeqCst),
            "result": format!("{result:?}")
        })
    );
    assert!(
        query_started_live.get(),
        "SQL started after the original deadline"
    );
    assert!(
        observed_rows.get() > 0,
        "recursive VM emitted no actual row"
    );
    assert!(
        interrupted.load(Ordering::SeqCst),
        "fixture did not execute an actual SQLite VM interruption"
    );
    assert!(
        matches!(result, Err(StoreError::BudgetExceeded)),
        "actual VM refusal: {result:?}"
    );
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}

#[test]
fn control_write_rust_unwind_rolls_back_and_restores_the_real_connection() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let mut insert = fixture.store.connection.prepare(PARTIAL_INSERT).unwrap();
    let mut exists = fixture.store.connection.prepare(PARTIAL_EXISTS).unwrap();
    assert!(fixture.store.connection.is_autocommit());
    let reached = AtomicBool::new(false);
    let target_started_live = AtomicBool::new(false);
    let expired_before_panic = AtomicBool::new(false);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(2);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let window = ControlWriteDeadline::new(
            &fixture.store.connection,
            deadline,
            fixture.authority.expires_at_unix_seconds(),
        )
        .unwrap();
        window.begin().unwrap();
        write_partial_job(&fixture, &window, &mut insert, &mut exists);
        reached.store(true, Ordering::SeqCst);
        target_started_live.store(Instant::now() < deadline, Ordering::SeqCst);
        wait_past_original_deadline(deadline);
        expired_before_panic.store(Instant::now() >= deadline, Ordering::SeqCst);
        // panic 发生在普通 Rust 调用栈，绝不跨 SQLite FFI 或在回调内重入。
        std::panic::panic_any(UNWIND_SENTINEL);
    }));
    let payload = result
        .as_ref()
        .err()
        .and_then(|value| value.downcast_ref::<&str>())
        .copied();
    eprintln!(
        "D44_UNWIND_STAGE {}",
        serde_json::json!({
            "original_window_ms": 2000,
            "elapsed_us": started.elapsed().as_micros(),
            "partial_write_witnessed": reached.load(Ordering::SeqCst),
            "target_started_live": target_started_live.load(Ordering::SeqCst),
            "expired_before_panic": expired_before_panic.load(Ordering::SeqCst),
            "exact_sentinel": payload == Some(UNWIND_SENTINEL)
        })
    );
    assert!(
        reached.load(Ordering::SeqCst),
        "unwind fixture never wrote inside transaction"
    );
    assert!(target_started_live.load(Ordering::SeqCst));
    assert!(expired_before_panic.load(Ordering::SeqCst));
    assert_eq!(
        payload,
        Some(UNWIND_SENTINEL),
        "preparation panic cannot prove unwind cleanup"
    );
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}

#[test]
fn control_write_expired_twenty_millisecond_window_refuses_before_a_transaction() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let deadline = Instant::now() + Duration::from_millis(20);
    wait_past_original_deadline(deadline);
    let result = ControlWriteDeadline::new(
        &fixture.store.connection,
        deadline,
        fixture.authority.expires_at_unix_seconds(),
    );
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}
