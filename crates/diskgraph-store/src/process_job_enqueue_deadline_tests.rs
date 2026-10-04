//! D44 入队原期限贯穿真实 SQLite BEGIN/最后 INSERT/COMMIT；无生产算法替身或源访问。
use crate::process_job_enqueue_fixture::ProcessEnqueueFixture;
use crate::{ControlStore, JobState, StoreError};
use diskgraph_core::{JobRequestAuthority, Permission};
use rusqlite::{Connection, ErrorCode};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

fn wait_past(deadline: Instant) {
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining + Duration::from_millis(20));
    }
}

fn writer_contention(running_merge: bool) {
    let mut fixture = ProcessEnqueueFixture::new();
    let original = running_merge.then(|| {
        let job = fixture
            .store
            .create_process_evidence_job(&fixture.input, &fixture.authority, 64)
            .unwrap()
            .unwrap();
        fixture
            .store
            .claim_job(&job.job_id, "actual-owner")
            .unwrap()
    });
    let before = fixture.persisted_state();
    let path = fixture.database_path();
    let held = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&held);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let locker = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.store(true, Ordering::SeqCst);
        ready_tx.send(()).unwrap();
        // 自动上限使旧实现也能退出；正常受限返回后由调用方立即释放，无永久等待。
        let _ = release_rx.recv_timeout(Duration::from_millis(1200));
        held.store(false, Ordering::SeqCst);
        connection.execute_batch("COMMIT").unwrap();
    });
    if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
        let _ = release_tx.send(());
        let joined = locker.join();
        panic!("external writer never acquired the real transaction: {error:?}; {joined:?}");
    }
    let started = Instant::now();
    let deadline = started + Duration::from_millis(400);
    // 同一请求准备已经消耗多数时间，不能从真正开始写时重建400ms。
    std::thread::sleep(Duration::from_millis(250));
    let remaining_at_call = deadline.saturating_duration_since(Instant::now());
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    let elapsed = started.elapsed();
    let returned_while_held = observed.load(Ordering::SeqCst);
    let _ = release_tx.send(());
    locker.join().unwrap();
    assert!(
        !remaining_at_call.is_zero(),
        "fixture expired before the actual enqueue call; remaining={remaining_at_call:?}"
    );
    assert!(
        matches!(result, Err(StoreError::BudgetExceeded)),
        "late queue admission/merge: elapsed={elapsed:?}, actual={result:?}"
    );
    assert!(returned_while_held, "waited for external writer release");
    assert!(
        elapsed < Duration::from_millis(550),
        "deadline refreshed: {elapsed:?}"
    );
    assert_eq!(
        fixture.persisted_state(),
        before,
        "persistent queue changed"
    );
    if let Some(original) = original {
        assert_eq!(original.state, JobState::Running);
        assert_eq!(fixture.store.job(&original.job_id).unwrap(), original);
    }
    fixture.assert_connection_restored(5000);
}

#[test]
fn process_enqueue_writer_wait_consumes_the_original_remaining_deadline() {
    writer_contention(false);
}

#[test]
fn process_enqueue_running_merge_cannot_wait_past_the_original_deadline() {
    writer_contention(true);
}

#[test]
fn process_enqueue_last_input_insert_expiry_rolls_back_all_durable_tables() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let reached = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&reached);
    let inserted = Arc::new(AtomicU8::new(0));
    let seen = Arc::clone(&inserted);
    let qualified = Arc::new(AtomicBool::new(false));
    let live = Arc::clone(&qualified);
    // 此窗口用于到达真实最后 INSERT，不是性能 SLA；注入等待仍消耗同一原期限。
    let deadline = Instant::now() + Duration::from_secs(2);
    fixture
        .store
        .connection
        .update_hook(Some(
            move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if action != rusqlite::hooks::Action::SQLITE_INSERT {
                    return;
                }
                match table {
                    "jobs" => {
                        seen.fetch_or(1, Ordering::SeqCst);
                    }
                    "job_request_authorities" => {
                        seen.fetch_or(2, Ordering::SeqCst);
                    }
                    "process_evidence_job_inputs" => {
                        live.store(
                            Instant::now() < deadline && seen.load(Ordering::SeqCst) == 3,
                            Ordering::SeqCst,
                        );
                        seen.fetch_or(4, Ordering::SeqCst);
                        mark.store(true, Ordering::SeqCst);
                        wait_past(deadline);
                    }
                    _ => {}
                }
            },
        ))
        .unwrap();
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    fixture
        .store
        .connection
        .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
        .unwrap();
    assert!(
        reached.load(Ordering::SeqCst),
        "never reached actual final input INSERT; actual={result:?}"
    );
    assert!(
        qualified.load(Ordering::SeqCst),
        "final INSERT must start before original expiry after both predecessor writes; actual={result:?}"
    );
    assert_eq!(
        inserted.load(Ordering::SeqCst),
        7,
        "all three actual writes required"
    );
    assert!(
        matches!(result, Err(StoreError::BudgetExceeded)),
        "late commit: {result:?}"
    );
    assert_eq!(
        fixture.persisted_state(),
        before,
        "partial admission persisted"
    );
    fixture.assert_connection_restored(731);
    // 旧可信入口仍可正常创建；不能继承上次请求的过期 handler/事务。
    let job = fixture
        .store
        .create_process_evidence_job(&fixture.input, &fixture.authority, 64)
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(
        fixture
            .store
            .process_evidence_job_input(&job.job_id)
            .unwrap(),
        fixture.input
    );
}

#[test]
fn process_enqueue_expired_quota_request_is_budget_error_without_any_mutation() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        0,
        Instant::now() - Duration::from_millis(1),
    );
    assert!(
        matches!(result, Err(StoreError::BudgetExceeded)),
        "actual={result:?}"
    );
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}

#[test]
fn process_enqueue_timely_create_and_merge_preserve_the_original_job() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let deadline = Instant::now() + Duration::from_millis(600);
    let first = fixture
        .store
        .create_process_evidence_job_until(&fixture.input, &fixture.authority, 64, deadline)
        .unwrap()
        .unwrap();
    assert_eq!(first.state, JobState::Queued);
    let before = fixture.persisted_state();
    let merged = fixture
        .store
        .create_process_evidence_job_until(&fixture.input, &fixture.authority, 64, deadline)
        .unwrap()
        .unwrap();
    assert_eq!(merged, first);
    assert_eq!(fixture.persisted_state(), before);
    wait_past(deadline);
    fixture.assert_connection_restored(731);
}

#[test]
fn process_enqueue_timely_quota_none_preserves_configuration_and_storage() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let deadline = Instant::now() + Duration::from_millis(250);
    let result = fixture
        .store
        .create_process_evidence_job_until(&fixture.input, &fixture.authority, 0, deadline)
        .unwrap();
    assert!(result.is_none());
    assert_eq!(fixture.persisted_state(), before);
    wait_past(deadline);
    fixture.assert_connection_restored(731);
}

#[test]
fn process_enqueue_host_busy_timeout_is_not_relabelled_as_request_expiry() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(40);
    let before = fixture.persisted_state();
    let connection = Connection::open(fixture.database_path()).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let deadline = Instant::now() + Duration::from_millis(600);
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    let finished_before_deadline = Instant::now() < deadline;
    // 断言前释放真实外部锁，即使回归失败也不遗留 owner 或永久阻塞。
    connection.execute_batch("ROLLBACK").unwrap();
    assert!(
        finished_before_deadline,
        "host's shorter timeout was expanded"
    );
    assert!(
        matches!(&result, Err(error) if error.is_busy()),
        "actual={result:?}"
    );
    assert_eq!(fixture.persisted_state(), before);
    wait_past(deadline);
    fixture.assert_connection_restored(40);
}

#[test]
fn process_enqueue_expired_original_authority_keeps_its_authorization_error() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let expired = JobRequestAuthority::authenticated_remote(
        fixture.authority.principal().clone(),
        "deadline-test-issuer",
        "http",
        vec![Permission::MetadataRead, Permission::IndexWrite],
        ControlStore::now_ms() / 1000 - 1,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_millis(250);
    let result =
        fixture
            .store
            .create_process_evidence_job_until(&fixture.input, &expired, 64, deadline);
    assert!(
        matches!(result, Err(StoreError::Conflict(_))),
        "actual={result:?}"
    );
    assert_eq!(fixture.persisted_state(), before);
    wait_past(deadline);
    fixture.assert_connection_restored(731);
}

#[test]
fn process_enqueue_late_real_commit_abort_is_not_rewritten_as_budget_error() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let before = fixture.persisted_state();
    let reached = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&reached);
    let deadline = Instant::now() + Duration::from_millis(150);
    fixture
        .store
        .connection
        .commit_hook(Some(move || {
            mark.store(true, Ordering::SeqCst);
            wait_past(deadline);
            true
        }))
        .unwrap();
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    fixture
        .store
        .connection
        .commit_hook(None::<fn() -> bool>)
        .unwrap();
    assert!(
        reached.load(Ordering::SeqCst),
        "real COMMIT hook not reached"
    );
    assert!(
        matches!(&result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == ErrorCode::ConstraintViolation),
        "SQLite commit refusal was masked: {result:?}"
    );
    assert_eq!(fixture.persisted_state(), before);
    fixture.assert_connection_restored(731);
}

#[test]
fn process_enqueue_writer_revocation_is_rechecked_after_transaction_acquisition() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    let connection = Connection::open(fixture.database_path()).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        connection
            .execute(
                "DELETE FROM grants WHERE principal_id=?1 AND scope_id=?2 AND permission=?3",
                rusqlite::params![
                    fixture.authority.principal().as_str(),
                    fixture.input.scope_id().as_str(),
                    Permission::IndexWrite.wire_name()
                ],
            )
            .unwrap(),
        1
    );
    let (release_tx, release_rx) = mpsc::channel();
    let locker = std::thread::spawn(move || {
        let _ = release_rx.recv_timeout(Duration::from_millis(100));
        connection.execute_batch("COMMIT").unwrap();
    });
    let deadline = Instant::now() + Duration::from_millis(600);
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    let _ = release_tx.send(());
    locker.join().unwrap();
    assert!(
        matches!(result, Err(StoreError::Conflict(_))),
        "actual={result:?}"
    );
    assert!(fixture.store.list_queued_jobs().unwrap().is_empty());
    for table in [
        "jobs",
        "job_request_authorities",
        "process_evidence_job_inputs",
        "process_job_failures",
    ] {
        let count: i64 = fixture
            .store
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "unauthorized rows in {table}");
    }
    wait_past(deadline);
    fixture.assert_connection_restored(731);
}
