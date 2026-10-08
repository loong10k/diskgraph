//! 使用真实控制库和真实竞争锁验证采样门禁；不依赖受管扫描 worker 安装。
use crate::scan_observation_guard::ScanObservationGuard;
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{BusinessError, PrincipalId};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

thread_local! {
    static SQL_FENCE: std::cell::RefCell<Option<std::sync::mpsc::Sender<()>>> = const { std::cell::RefCell::new(None) };
    static CONTROL_WAIT: std::cell::RefCell<Option<std::sync::mpsc::Sender<()>>> = const { std::cell::RefCell::new(None) };
}

/// 仅在实际 WouldBlock 后通知本测试线程绑定的竞争观察者；不等待、不读写数据库。
pub(super) fn waiting_for_control() {
    CONTROL_WAIT.with(|slot| {
        if let Some(sender) = slot.borrow_mut().take() {
            let _ = sender.send(());
        }
    });
}

fn fixture() -> (tempfile::TempDir, Engine, diskgraph_store::JobRecord) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("observation-deadline").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let scope = engine.register_scope(&root, &actor, &policy).unwrap();
    let queued = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let running = engine
        .control()
        .unwrap()
        .claim_job_once(&queued.job_id, "original-owner")
        .unwrap();
    (dir, engine, running)
}

#[test]
fn held_control_lock_cannot_extend_the_original_observation_budget() {
    let (_dir, mut engine, job) = fixture();
    engine.scan_budget.max_duration_ms = 50;
    let held = engine.control().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let cancel = AtomicBool::new(false);
            let guard = ScanObservationGuard::new(&engine, &job, None, &cancel, Instant::now());
            tx.send(guard.check_now()).unwrap();
        });
        let before_release = rx.recv_timeout(Duration::from_millis(300));
        drop(held);
        // 旧实现必须先释放真实竞争锁，才能结束线程；红灯也不能遗留阻塞线程。
        let returned_before_release = before_release.is_ok();
        let result =
            before_release.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(
            returned_before_release,
            "observation waited beyond budget for the held lock"
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "{result:?}"
        );
    });
}

#[test]
fn cancellation_and_original_stop_reason_interrupt_an_actual_control_wait() {
    use crate::job_execution_stop_reason::JobExecutionStopReason;
    use std::sync::atomic::Ordering;

    for original_reason in [false, true] {
        let (_dir, engine, job) = fixture();
        let held = engine.control().unwrap();
        let cancel = AtomicBool::new(false);
        let reason = JobExecutionStopReason::new();
        let (wait_tx, wait_rx) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|threads| {
            threads.spawn(|| {
                CONTROL_WAIT.with(|slot| *slot.borrow_mut() = Some(wait_tx));
                let guard = ScanObservationGuard::new(&engine, &job, None, &cancel, Instant::now())
                    .with_stop_reason(&reason);
                tx.send(guard.check_now()).unwrap();
            });
            let actually_waiting = wait_rx.recv_timeout(Duration::from_secs(2)).is_ok();
            if original_reason {
                reason.record(BusinessError::PermissionDenied.into());
            }
            cancel.store(true, Ordering::SeqCst);
            let before_release = rx.recv_timeout(Duration::from_millis(300));
            drop(held);
            let returned_before_release = before_release.is_ok();
            let result =
                before_release.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(
                actually_waiting,
                "must observe the real contended lock before cancellation"
            );
            assert!(
                returned_before_release,
                "cancelled observation kept waiting for the held control lock"
            );
            let expected = if original_reason {
                BusinessError::PermissionDenied
            } else {
                BusinessError::Conflict
            };
            assert!(
                matches!(result, Err(EngineError::Business(actual)) if actual == expected),
                "cancellation lost its original reason"
            );
        });
    }
}

#[test]
fn competing_sqlite_writer_cannot_extend_the_original_observation_budget() {
    let (dir, mut engine, job) = fixture();
    engine.scan_budget.max_duration_ms = 50;
    let writer =
        rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|threads| {
        threads.spawn(|| {
            let cancel = AtomicBool::new(false);
            let guard = ScanObservationGuard::new(&engine, &job, None, &cancel, Instant::now());
            tx.send(guard.check_now()).unwrap();
        });
        let before_release = rx.recv_timeout(Duration::from_millis(300));
        writer.execute_batch("ROLLBACK").unwrap();
        let returned_before_release = before_release.is_ok();
        let result =
            before_release.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap());
        assert!(
            returned_before_release,
            "observation waited beyond budget for the SQLite writer"
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "{result:?}"
        );
        // 原连接的 deadline 配置必须恢复，后续真实检查仍能取得 fence。
        engine
            .control()
            .unwrap()
            .with_job_fence(&job.job_id, &job.owner, job.fencing_token, || Ok(()))
            .unwrap();
    });
}

#[test]
fn live_guard_still_refuses_persistent_scope_revocation() {
    let (_dir, engine, job) = fixture();
    let cancel = AtomicBool::new(false);
    let guard = ScanObservationGuard::new(&engine, &job, None, &cancel, Instant::now());
    guard.check_now().unwrap();
    engine
        .control()
        .unwrap()
        .revoke_scope(&job.scope_id)
        .unwrap();
    assert!(matches!(
        guard.check_now(),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn bounded_observation_preserves_cancel_permission_and_fence_denials() {
    for mutation in 0..3 {
        let (_dir, engine, mut job) = fixture();
        match mutation {
            0 => {
                engine
                    .control()
                    .unwrap()
                    .request_cancel(&job.job_id)
                    .unwrap();
            }
            1 => {
                engine
                    .control()
                    .unwrap()
                    .revoke_grant(
                        &job.principal,
                        &diskgraph_core::Permission::IndexWrite,
                        &job.scope_id,
                    )
                    .unwrap();
            }
            _ => {
                job.fencing_token += 1;
            }
        }
        let cancel = AtomicBool::new(false);
        let guard = ScanObservationGuard::new(&engine, &job, None, &cancel, Instant::now());
        let result = guard.check_now();
        match mutation {
            0 => assert!(
                matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
                "{result:?}"
            ),
            1 => assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "{result:?}"
            ),
            _ => assert!(
                matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
                "{result:?}"
            ),
        }
    }
}

/// 仅通知测试线程已完成只读检查、即将进入原 fence 事务。
pub(super) fn entering_sql_fence() {
    SQL_FENCE.with(|slot| {
        if let Some(sender) = slot.borrow_mut().take() {
            let _ = sender.send(());
        }
    });
}

#[test]
fn cancellation_interrupts_sqlite_fence_wait_and_preserves_keeper_reason() {
    use crate::job_execution_stop_reason::JobExecutionStopReason;
    use std::sync::atomic::Ordering;
    for original_reason in [false, true] {
        let (dir, mut engine, job) = fixture();
        engine.scan_budget.max_duration_ms = 2_000;
        let writer =
            rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let cancel = AtomicBool::new(false);
        let reason = JobExecutionStopReason::new();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|threads| {
            threads.spawn(|| {
                SQL_FENCE.with(|slot| *slot.borrow_mut() = Some(entered_tx));
                let guard = ScanObservationGuard::new(&engine, &job, None, &cancel, Instant::now())
                    .with_stop_reason(&reason);
                tx.send(guard.check_now()).unwrap();
            });
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if original_reason {
                reason.record(BusinessError::PermissionDenied.into());
            }
            cancel.store(true, Ordering::SeqCst);
            let before_release = rx.recv_timeout(Duration::from_millis(300));
            writer.execute_batch("ROLLBACK").unwrap();
            let promptly = before_release.is_ok();
            let result =
                before_release.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap());
            assert!(
                promptly,
                "cancelled scan waited for SQLite writer release: {result:?}"
            );
            let expected = if original_reason {
                BusinessError::PermissionDenied
            } else {
                BusinessError::Conflict
            };
            assert!(matches!(result, Err(EngineError::Business(actual)) if actual == expected));
            engine
                .control()
                .unwrap()
                .with_job_fence(&job.job_id, &job.owner, job.fencing_token, || Ok(()))
                .unwrap();
        });
    }
}
