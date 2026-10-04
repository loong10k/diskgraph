//! D44 真实提交竞争、VM 中断与 Rust unwind 清理，不以事务内部状态断言替代持久结果。
use crate::StoreError;
use crate::control_write_deadline::ControlWriteDeadline;
use crate::process_job_enqueue_fixture::ProcessEnqueueFixture;
use rusqlite::Connection;
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
        "never reached the actual final input INSERT"
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

fn insert_partial_job(fixture: &ProcessEnqueueFixture) {
    fixture.store.connection.execute(
        "INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,owner,principal) VALUES('guard-partial',?1,'process_evidence','queued',1,1,'',?2)",
        rusqlite::params![fixture.input.scope_id().as_str(), fixture.authority.principal().as_str()],
    ).unwrap();
}

#[test]
fn control_write_real_vm_interrupt_clears_progress_before_partial_transaction_rollback() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let window = ControlWriteDeadline::new(
        &fixture.store.connection,
        Instant::now() + Duration::from_millis(20),
        fixture.authority.expires_at_unix_seconds(),
    )
    .unwrap();
    window.begin().unwrap();
    insert_partial_job(&fixture);
    let interrupted = AtomicBool::new(false);
    let result = window.statement(|| -> crate::Result<i64> {
        let actual = fixture.store.connection.query_row(
            "WITH RECURSIVE work(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM work WHERE n<1000000000) SELECT sum(n) FROM work",
            [], |row| row.get(0),
        );
        // 在普通 Rust closure 观察原始 SQLite 错误，不能由进入 SQL 前的到期拒绝充数。
        if matches!(&actual, Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::OperationInterrupted) {
            interrupted.store(true, Ordering::SeqCst);
        }
        Ok(actual?)
    });
    let result = window.finish(result);
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
    let reached = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_millis(20);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let window = ControlWriteDeadline::new(
            &fixture.store.connection,
            deadline,
            fixture.authority.expires_at_unix_seconds(),
        )
        .unwrap();
        window.begin().unwrap();
        insert_partial_job(&fixture);
        reached.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(40));
        // panic 发生在普通 Rust 调用栈，绝不跨 SQLite FFI 或在回调内重入。
        panic!("actual Rust consumer unwind after partial write");
    }));
    assert!(
        reached.load(Ordering::SeqCst),
        "unwind fixture never wrote inside transaction"
    );
    assert!(result.is_err());
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}
