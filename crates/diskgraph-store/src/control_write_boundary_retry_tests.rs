//! D44 封闭 BEGIN/COMMIT 重试的真实 SQLite 回归；来源：原生 Rust 控制入队协议。
use crate::control_write_deadline::ControlWriteDeadline;
use crate::process_job_enqueue_fixture::ProcessEnqueueFixture;
use crate::{JobKind, JobState, StoreError};
use diskgraph_core::{QueryBudget, QueryReadBudget};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use rusqlite::{Connection, ErrorCode};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

fn wait_past(deadline: Instant) {
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining + Duration::from_millis(20));
    }
}

// rusqlite 0.40 的 COMMIT 映射 Unknown；本窗口唯一该类事务 literal 为生产 COMMIT。
fn observe_commit_preparations(fixture: &ProcessEnqueueFixture) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let mark = Arc::clone(&count);
    fixture
        .store
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Unknown
                }
            ) {
                mark.fetch_add(1, Ordering::SeqCst);
            }
            Authorization::Allow
        }))
        .unwrap();
    count
}

#[test]
fn control_write_zero_host_busy_refuses_a_real_competing_writer_without_waiting_for_release() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(0);
    let before = fixture.persisted_state();
    let other = Connection::open(fixture.database_path()).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let window = ControlWriteDeadline::new(
        &fixture.store.connection,
        deadline,
        fixture.authority.expires_at_unix_seconds(),
    )
    .unwrap();
    let result = window.begin();
    let result = window.finish(result);
    let original_deadline_live = Instant::now() < deadline;
    // 锁仍由当前线程持有，因此成功/等待释放都不能伪装成宿主0ms的正常BUSY。
    assert!(!other.is_autocommit());
    other.execute_batch("ROLLBACK").unwrap();
    let after = fixture.persisted_state();
    wait_past(deadline);
    fixture.assert_connection_restored(0);
    assert!(original_deadline_live);
    assert!(
        matches!(&result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _))) if code.code == ErrorCode::DatabaseBusy && code.extended_code == rusqlite::ffi::SQLITE_BUSY),
        "host0 actual result={result:?}"
    );
    assert_eq!(after, before);
}

#[test]
fn process_enqueue_zero_host_busy_still_creates_and_merges_without_contention() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(0);
    let deadline = Instant::now() + Duration::from_secs(2);
    let first = fixture
        .store
        .create_process_evidence_job_until(&fixture.input, &fixture.authority, 64, deadline)
        .unwrap()
        .unwrap();
    let before_merge = fixture.persisted_state();
    let merged = fixture
        .store
        .create_process_evidence_job_until(&fixture.input, &fixture.authority, 64, deadline)
        .unwrap()
        .unwrap();
    let original_deadline_live = Instant::now() < deadline;
    assert_eq!(first.kind, JobKind::ProcessEvidence);
    assert_eq!(first.state, JobState::Queued);
    assert_eq!(merged, first);
    assert_eq!(fixture.persisted_state(), before_merge);
    assert_eq!(
        fixture
            .store
            .process_evidence_job_input(&first.job_id)
            .unwrap(),
        fixture.input
    );
    wait_past(deadline);
    fixture.assert_connection_restored(0);
    assert!(original_deadline_live);
}

// 实际 rollback-journal 的共享读锁只阻塞 COMMIT；不替换生产连接或事务算法。
fn held_reader(
    fixture: &ProcessEnqueueFixture,
) -> (
    std::thread::JoinHandle<()>,
    mpsc::Sender<()>,
    Arc<AtomicBool>,
) {
    let path = fixture.database_path();
    let held = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&held);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch("BEGIN").unwrap();
        let jobs: i64 = connection
            .query_row("SELECT count(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(jobs, 0);
        observed.store(true, Ordering::SeqCst);
        ready_tx.send(()).unwrap();
        if release_rx.recv_timeout(Duration::from_secs(2)).is_ok() {
            std::thread::sleep(Duration::from_millis(100));
        }
        observed.store(false, Ordering::SeqCst);
        connection.execute_batch("ROLLBACK").unwrap();
    });
    if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(5)) {
        let _ = release_tx.send(());
        let joined = reader.join();
        panic!("reader did not acquire its actual shared lock: {error:?}; {joined:?}");
    }
    (reader, release_tx, held)
}

#[test]
fn process_enqueue_commit_contention_writes_its_fixed_input_only_once() {
    let mut fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    fixture
        .store
        .connection
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    let (reader, release_tx, held) = held_reader(&fixture);
    let preparations = observe_commit_preparations(&fixture);
    let signal = release_tx.clone();
    let observed = Arc::clone(&held);
    let input_while_held = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&input_while_held);
    let inserts = Arc::new([
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ]);
    let counts = Arc::clone(&inserts);
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
                        counts[0].fetch_add(1, Ordering::SeqCst);
                    }
                    "job_request_authorities" => {
                        counts[1].fetch_add(1, Ordering::SeqCst);
                    }
                    "process_evidence_job_inputs" => {
                        counts[2].fetch_add(1, Ordering::SeqCst);
                        mark.store(observed.load(Ordering::SeqCst), Ordering::SeqCst);
                        let _ = signal.send(());
                    }
                    _ => {}
                }
            },
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = fixture.store.create_process_evidence_job_until(
        &fixture.input,
        &fixture.authority,
        64,
        deadline,
    );
    let _ = release_tx.send(());
    reader.join().unwrap();
    // 合并前关闭观察，不能把后续正常 COMMIT 当作首次竞争的重试见证。
    let first_commit_preparations = preparations.load(Ordering::SeqCst);
    fixture
        .store
        .connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    let first = result.unwrap().unwrap();
    let before_merge = fixture.persisted_state();
    let merged = fixture
        .store
        .create_process_evidence_job_until(&fixture.input, &fixture.authority, 64, deadline)
        .unwrap()
        .unwrap();
    fixture
        .store
        .connection
        .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>)
        .unwrap();
    assert!(input_while_held.load(Ordering::SeqCst));
    assert!(
        first_commit_preparations >= 2,
        "no real COMMIT retry observed: {first_commit_preparations}"
    );
    assert_eq!(first.state, JobState::Queued);
    assert_eq!(merged, first);
    assert_eq!(fixture.persisted_state(), before_merge);
    for count in inserts.iter() {
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "boundary repeated a durable admission INSERT"
        );
    }
    assert_eq!(
        fixture
            .store
            .process_evidence_job_input(&first.job_id)
            .unwrap(),
        fixture.input
    );
    wait_past(deadline);
    fixture.assert_connection_restored(731);
}

#[test]
fn control_write_commit_contention_does_not_replay_a_completed_budgeted_statement() {
    let fixture = ProcessEnqueueFixture::new();
    fixture.set_busy_timeout(731);
    fixture
        .store
        .connection
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    let (reader, release_tx, held) = held_reader(&fixture);
    let preparations = observe_commit_preparations(&fixture);
    let raw_len = fixture.input.scope_id().as_str().len();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut reads = QueryReadBudget::new(
        QueryBudget {
            max_nodes: 1,
            max_response_bytes: raw_len,
            ..QueryBudget::default()
        },
        deadline,
    )
    .unwrap();
    let calls = Cell::new(0);
    let window = ControlWriteDeadline::new(
        &fixture.store.connection,
        deadline,
        fixture.authority.expires_at_unix_seconds(),
    )
    .unwrap();
    let result = (|| {
        window.begin()?;
        window.statement(|| -> crate::Result<()> {
            calls.set(calls.get() + 1);
            if !reads.admit(1, 0, raw_len) {
                return Err(StoreError::BudgetExceeded);
            }
            let scope: String = fixture.store.connection.query_row(
                "SELECT scope_id FROM scopes WHERE scope_id=?1", [fixture.input.scope_id().as_str()], |row| row.get(0),
            )?;
            assert_eq!(scope, fixture.input.scope_id().as_str());
            fixture.store.connection.execute(
                "INSERT INTO jobs(job_id,scope_id,kind,state,created_at_unix_ms,heartbeat_unix_ms,owner,principal) VALUES('boundary-once',?1,'index','queued',1,1,'',?2)",
                rusqlite::params![scope,fixture.authority.principal().as_str()],
            )?;
            Ok(())
        })?;
        assert!(held.load(Ordering::SeqCst));
        let _ = release_tx.send(());
        window.commit()
    })();
    let result = window.finish(result);
    let _ = release_tx.send(());
    reader.join().unwrap();
    fixture
        .store
        .connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    result.unwrap();
    assert!(
        preparations.load(Ordering::SeqCst) >= 2,
        "no real COMMIT retry observed"
    );
    assert_eq!(
        calls.get(),
        1,
        "COMMIT re-entered completed admission consumer"
    );
    assert_eq!(reads.nodes_read(), 1);
    assert_eq!(reads.remaining_raw_bytes(), 0);
    let jobs: i64 = fixture
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM jobs WHERE job_id='boundary-once'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    wait_past(deadline);
    fixture.assert_connection_restored(731);
}
