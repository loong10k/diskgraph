//! 原 scan guard、keeper 与 heartbeat 的实际失败不能被一位停止标志抹去。
//! 来源：原生 Rust 持久任务、真实 SQLite 写锁和实时 grant 撤销。

use crate::EngineError;
use crate::git_evidence_fixture::GitEvidenceFixture;
use crate::job_stop_cause_fixture::{
    assert_failed_without_caller_cancel, enqueue_scan, lease_live, qualify_claim, sqlite_busy,
    withdraw,
};
use crate::job_stop_cause_hooks as hooks;
use diskgraph_core::{BusinessError, Permission};
use diskgraph_store::StoreError;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn scan_guard_keeps_its_actual_permission_denied_before_cooperative_stop() {
    let f = Arc::new(GitEvidenceFixture::new());
    let job = enqueue_scan(&f);
    let (paused, keeper_paused) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let fixture = f.clone();
    hooks::before_keeper(&job.job_id, move |claimed| {
        qualify_claim(&fixture, claimed, "guard-cause");
        paused.send(claimed.fencing_token).unwrap();
        // 超时只是避免回归自身永久等待；资格在真实 join 之后逐项断言。
        let _ = resumed.recv_timeout(Duration::from_secs(20));
    });
    let fixture = f.clone();
    let spawn_seen = Arc::new(AtomicBool::new(false));
    let spawned = spawn_seen.clone();
    hooks::at_spawn(&job.job_id, move || {
        keeper_paused.recv_timeout(Duration::from_secs(10)).unwrap();
        spawned.store(true, Ordering::SeqCst);
        withdraw(&fixture, Permission::IndexWrite);
    });
    let denied_seen = Arc::new(AtomicBool::new(false));
    let denied = denied_seen.clone();
    let release = resume.clone();
    hooks::at_scan_error(&job.job_id, move |error| {
        denied.store(
            matches!(
                error,
                EngineError::Business(BusinessError::PermissionDenied)
            ),
            Ordering::SeqCst,
        );
        let _ = release.send(());
    });
    hooks::at_outcome(&job.job_id, move |_| {
        let _ = resume.send(());
    });
    let result = f.engine.run_job(&job.job_id, "guard-cause");
    hooks::clear();
    assert!(spawn_seen.load(Ordering::SeqCst));
    assert!(
        denied_seen.load(Ordering::SeqCst),
        "must see real guard Denied before its stop bit"
    );
    assert_failed_without_caller_cancel(&f, &job.job_id, "guard-cause");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "actual scan guard error was replaced: {result:?}"
    );
}

#[test]
fn keeper_real_sqlite_busy_is_retained_instead_of_a_derived_scan_conflict() {
    let f = Arc::new(GitEvidenceFixture::new());
    let job = enqueue_scan(&f);
    let (paused, keeper_paused) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let (failed, keeper_failed) = std::sync::mpsc::channel();
    let fixture = f.clone();
    hooks::before_keeper(&job.job_id, move |claimed| {
        qualify_claim(&fixture, claimed, "keeper-busy");
        paused.send(claimed.fencing_token).unwrap();
        let _ = resumed.recv_timeout(Duration::from_secs(20));
    });
    hooks::at_keeper_error(&job.job_id, move |claimed, error| {
        failed
            .send((
                claimed.fencing_token,
                sqlite_busy(error),
                lease_live(claimed),
            ))
            .unwrap();
    });
    let path = f.engine.data_dir().join("diskgraph-control.sqlite");
    let busy_seen = Arc::new(AtomicBool::new(false));
    let observed = busy_seen.clone();
    hooks::at_spawn(&job.job_id, move || {
        let fence = keeper_paused.recv_timeout(Duration::from_secs(10)).unwrap();
        let writer = rusqlite::Connection::open(path).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(
            !writer.is_autocommit(),
            "must hold a real external writer transaction"
        );
        resume.send(()).unwrap();
        let proof = keeper_failed.recv_timeout(Duration::from_secs(12));
        // 实际 SQL 已返回后先释放外部锁，再检查资格，不让失败遗留数据库竞争。
        writer.execute_batch("ROLLBACK").unwrap();
        let (actual_fence, busy, live) = proof.unwrap();
        assert_eq!(actual_fence, fence);
        assert!(live, "original lease expired before the target SQL failure");
        observed.store(busy, Ordering::SeqCst);
    });
    let started = Instant::now();
    let result = f.engine.run_job(&job.job_id, "keeper-busy");
    hooks::clear();
    assert!(
        busy_seen.load(Ordering::SeqCst),
        "must witness actual SQLite BUSY(5)"
    );
    assert!(started.elapsed() < Duration::from_millis(f.engine.scan_budget.max_duration_ms));
    assert_failed_without_caller_cancel(&f, &job.job_id, "keeper-busy");
    assert!(
        result.as_ref().is_err_and(sqlite_busy),
        "keeper original error was lost: {result:?}"
    );
}

#[test]
fn scan_heartbeat_keeps_a_real_sqlite_error_instead_of_guessing_stale_owner() {
    let f = Arc::new(GitEvidenceFixture::new());
    let job = enqueue_scan(&f);
    let (paused, keeper_paused) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let fixture = f.clone();
    hooks::before_keeper(&job.job_id, move |claimed| {
        qualify_claim(&fixture, claimed, "heartbeat-busy");
        paused.send(()).unwrap();
        let _ = resumed.recv_timeout(Duration::from_secs(25));
    });
    let writer = Rc::new(RefCell::new(None));
    let held = writer.clone();
    let path = f.engine.data_dir().join("diskgraph-control.sqlite");
    hooks::at_spawn(&job.job_id, move || {
        keeper_paused.recv_timeout(Duration::from_secs(10)).unwrap();
        // 不修改 now_ms/心跳周期/租约：只让真实启动后的原 5s 心跳阶段到达。
        let began = Instant::now();
        while began.elapsed() < Duration::from_millis(5001) {
            std::thread::sleep(Duration::from_millis(2));
        }
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(!connection.is_autocommit());
        *held.borrow_mut() = Some(connection);
    });
    let busy_seen = Arc::new(AtomicBool::new(false));
    let observed = busy_seen.clone();
    let held = writer.clone();
    let release = resume.clone();
    let fixture = f.clone();
    let id = job.job_id.clone();
    hooks::at_heartbeat_error(&job.job_id, move |error| {
        let running = fixture.engine.job_status(&id).unwrap();
        assert_eq!(running.owner, "heartbeat-busy");
        assert!(
            lease_live(&running),
            "lease must remain live at the actual heartbeat failure"
        );
        observed.store(
            matches!(error,
            StoreError::Sqlite(rusqlite::Error::SqliteFailure(code, _))
            if code.code == rusqlite::ErrorCode::DatabaseBusy && code.extended_code == 5),
            Ordering::SeqCst,
        );
        held.borrow_mut()
            .take()
            .unwrap()
            .execute_batch("ROLLBACK")
            .unwrap();
        let _ = release.send(());
    });
    let held = writer.clone();
    hooks::at_outcome(&job.job_id, move |_| {
        if let Some(connection) = held.borrow_mut().take() {
            connection.execute_batch("ROLLBACK").unwrap();
        }
        let _ = resume.send(());
    });
    let result = f.engine.run_job(&job.job_id, "heartbeat-busy");
    hooks::clear();
    assert!(writer.borrow().is_none());
    assert!(
        busy_seen.load(Ordering::SeqCst),
        "must reach actual heartbeat SQL BUSY(5)"
    );
    assert_failed_without_caller_cancel(&f, &job.job_id, "heartbeat-busy");
    assert!(
        result.as_ref().is_err_and(sqlite_busy),
        "heartbeat original error was replaced: {result:?}"
    );
}

#[test]
fn keeper_sqlite_busy_after_final_native_validation_is_not_permission_denied() {
    let f = Arc::new(GitEvidenceFixture::new());
    let job = enqueue_scan(&f);
    let (paused, keeper_paused) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    let (failed, keeper_failed) = std::sync::mpsc::channel();
    let fixture = f.clone();
    hooks::before_keeper(&job.job_id, move |claimed| {
        qualify_claim(&fixture, claimed, "final-gate-busy");
        paused.send(claimed.clone()).unwrap();
        let _ = resumed.recv_timeout(Duration::from_secs(20));
    });
    hooks::at_keeper_error(&job.job_id, move |claimed, error| {
        // 只借用实际错误和已认领记录，观察回调不读数据库或等待。
        failed
            .send((
                claimed.fencing_token,
                sqlite_busy(error),
                lease_live(claimed),
            ))
            .unwrap();
    });
    let phase_seen = Arc::new(AtomicBool::new(false));
    let reached = phase_seen.clone();
    let busy_seen = Arc::new(AtomicBool::new(false));
    let observed = busy_seen.clone();
    let fixture = f.clone();
    let id = job.job_id.clone();
    hooks::at_final_scan_validation(&job.job_id, move || {
        let claimed = keeper_paused.recv_timeout(Duration::from_secs(10)).unwrap();
        let before = fixture.engine.job_status(&id).unwrap();
        assert_eq!(before.owner, claimed.owner);
        assert_eq!(before.fencing_token, claimed.fencing_token);
        assert_eq!(before.state, diskgraph_store::JobState::Running);
        assert!(lease_live(&before));
        let stage = format!("{id}:{}", claimed.fencing_token);
        let graph =
            rusqlite::Connection::open(fixture.engine.data_dir().join("diskgraph.sqlite")).unwrap();
        let staged: i64 = graph
            .query_row(
                "SELECT COUNT(*) FROM scan_staging WHERE job_id=?1",
                [&stage],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            staged > 0,
            "must pass real staging before the final native validation"
        );
        fixture.assert_no_git_publication();
        reached.store(true, Ordering::SeqCst);

        let writer =
            rusqlite::Connection::open(fixture.engine.data_dir().join("diskgraph-control.sqlite"))
                .unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert!(
            !writer.is_autocommit(),
            "must hold the real external writer"
        );
        resume.send(()).unwrap();
        let proof = keeper_failed.recv_timeout(Duration::from_secs(12));
        // 原 5s SQLite 写锁等待实际结束后先释放外部事务，再检查资格并恢复发布门禁。
        writer.execute_batch("ROLLBACK").unwrap();
        let (actual_fence, busy, live) = proof.unwrap();
        assert_eq!(actual_fence, claimed.fencing_token);
        assert!(busy, "must reach the keeper's actual SQLite BUSY(5)");
        assert!(live, "original lease must remain live at the actual error");
        let control = fixture.engine.control_store().unwrap();
        let running = control.job(&id).unwrap();
        assert_eq!(running.owner, claimed.owner);
        assert_eq!(running.fencing_token, claimed.fencing_token);
        assert_eq!(running.state, diskgraph_store::JobState::Running);
        assert!(lease_live(&running));
        assert!(!control.scope_revoked(&fixture.scope).unwrap());
        assert_eq!(
            control
                .live_permission(&fixture.actor, &Permission::IndexWrite, &fixture.scope)
                .unwrap(),
            Some(true)
        );
        assert!(
            !control
                .cancellation_requested(&id, claimed.fencing_token)
                .unwrap()
        );
        drop(control);
        assert!(
            fixture
                .engine
                .cancellations()
                .unwrap()
                .get(&id)
                .unwrap()
                .load(Ordering::SeqCst)
        );
        observed.store(busy, Ordering::SeqCst);
    });
    let started = Instant::now();
    let result = f.engine.run_job(&job.job_id, "final-gate-busy");
    hooks::clear();
    assert!(
        phase_seen.load(Ordering::SeqCst),
        "must reach final native validation before the graph lock"
    );
    assert!(busy_seen.load(Ordering::SeqCst));
    assert!(started.elapsed() < Duration::from_millis(f.engine.scan_budget.max_duration_ms));
    assert_failed_without_caller_cancel(&f, &job.job_id, "final-gate-busy");
    let graph = rusqlite::Connection::open(f.engine.data_dir().join("diskgraph.sqlite")).unwrap();
    let stage = format!(
        "{}:{}",
        job.job_id,
        f.engine.job_status(&job.job_id).unwrap().fencing_token
    );
    for table in [
        "scan_staging",
        "scan_staging_unix_observations",
        "scan_staging_search",
    ] {
        let remaining: i64 = graph
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE job_id=?1"),
                [&stage],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "failed scan must clear {table}");
    }
    assert!(
        result.as_ref().is_err_and(sqlite_busy),
        "final publication gate replaced the actual keeper cause: {result:?}"
    );
}
