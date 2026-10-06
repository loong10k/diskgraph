//! D37 有界调度页回归；来源：公开控制库入队、真实多主体任务与实际扫描。
//! 验证一次 tick 最多处理 64 个候选，而不是全队列授权清理后只执行一个任务。

use super::run_one_queued;
use crate::{Engine, EngineConfig};
use diskgraph_core::PrincipalId;
use diskgraph_store::{JobKind, JobState};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn strict_tick_bounds_candidate_work_and_advances_past_legacy_jobs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file.txt"), b"real scan input").unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: temp.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let actor = PrincipalId::new("bounded-runner-admin").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let mut legacy = Vec::new();
    for position in 0..65 {
        // 每个真实主体各有一个合法旧格式记录，避免同主体合并掩盖队列宽度。
        let principal = PrincipalId::new(format!("bounded-legacy-{position}")).unwrap();
        legacy.push(
            engine
                .control_store()
                .unwrap()
                .create_job(&scope, JobKind::Index, &principal)
                .unwrap(),
        );
    }
    // 使用真实创建时间安排下一页，不修改数据库排序键；避免同毫秒随机 ID 顺序。
    let last_created = legacy
        .iter()
        .map(|job| job.created_at_unix_ms)
        .max()
        .unwrap();
    let wait_limit = Instant::now() + Duration::from_secs(2);
    while SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        <= u128::from(last_created)
    {
        assert!(
            Instant::now() < wait_limit,
            "clock did not advance after fixture enqueue"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let valid = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let before = engine.queued_jobs().unwrap();
    assert_eq!(
        before.len(),
        66,
        "compatibility list must retain the full queue"
    );
    assert_eq!(before.last().unwrap().job_id, valid.job_id);
    let stop = AtomicBool::new(false);
    let first = run_one_queued(&engine, "bounded-owner", &stop, true).unwrap();
    let failed = legacy
        .iter()
        .filter(|job| engine.job_status(&job.job_id).unwrap().state == JobState::Failed)
        .count();
    let remaining = engine.queued_jobs().unwrap();
    eprintln!(
        "first strict tick: failed legacy={failed}, remaining={}, completed={:?}",
        remaining.len(),
        first.as_ref().map(|job| &job.job_id)
    );
    assert!(
        first.is_none(),
        "first page must not reach the valid job beyond 64 candidates"
    );
    assert_eq!(failed, 64, "strict claim settles only the admitted page");
    assert_eq!(remaining.len(), 2);
    assert!(engine.latest_revision(&scope).unwrap().is_none());
    let second = run_one_queued(&engine, "bounded-owner", &stop, true)
        .unwrap()
        .unwrap();
    assert_eq!(second.job_id, valid.job_id);
    assert_eq!(second.state, JobState::Completed);
    assert!(
        legacy
            .iter()
            .all(|job| engine.job_status(&job.job_id).unwrap().state == JobState::Failed)
    );
    assert!(engine.queued_jobs().unwrap().is_empty());
    assert!(engine.latest_revision(&scope).unwrap().is_some());
}

#[test]
fn stop_preserves_original_background_panic_payload() {
    let temp = tempfile::tempdir().unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: temp.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let runner = super::JobRunner {
        engine,
        stop: Arc::new(AtomicBool::new(false)),
        owner: "panic-owner".into(),
        require_authority: true,
        #[cfg(windows)]
        probe_recovery: None,
        worker: Some(std::thread::spawn(|| {
            std::panic::panic_any(String::from("original-background-panic"));
        })),
    };
    let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner.stop()));
    let payload = observed.expect_err("runner must not discard the original worker panic");
    assert_eq!(
        payload.downcast_ref::<String>().unwrap(),
        "original-background-panic"
    );
}

#[cfg(windows)]
#[test]
fn managed_runner_rejects_recovery_from_another_pool_before_start() {
    let temp = tempfile::tempdir().unwrap();
    let (host, original) = crate::ProbeHost::new(1).unwrap();
    let (engine, _) = Engine::open_with_process_hosts(
        EngineConfig {
            data_dir: temp.path().join("data"),
            ..EngineConfig::default()
        },
        None,
        host,
    )
    .unwrap();
    let (_, other) = crate::ProbeHost::new(1).unwrap();
    let engine = Arc::new(engine);
    let wrong =
        super::JobRunner::start_with_probe_recovery(Arc::clone(&engine), true, Arc::new(other));
    assert!(matches!(
        wrong,
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::InvalidArgument
        ))
    ));
    assert_eq!(original.occupied_slots().unwrap(), 0);
    let recovery = Arc::new(original);
    let runner =
        super::JobRunner::start_with_probe_recovery(engine, true, Arc::clone(&recovery)).unwrap();
    runner.stop_and_join().unwrap();
    assert!(recovery.drain().unwrap());
}

#[cfg(windows)]
#[test]
fn busy_managed_runner_does_not_claim_or_fail_queued_job() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let (host, recovery) = crate::ProbeHost::new(1).unwrap();
    let (engine, _) = Engine::open_with_process_hosts(
        EngineConfig {
            data_dir: temp.path().join("data"),
            ..EngineConfig::default()
        },
        None,
        host,
    )
    .unwrap();
    let engine = Arc::new(engine);
    let actor = PrincipalId::new("managed-busy-admin").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let guard = engine.runner_admission.lock().unwrap();
    let other_engine = Arc::clone(&engine);
    let observed = std::thread::spawn(move || {
        run_one_queued(
            &other_engine,
            "second-runner",
            &AtomicBool::new(false),
            true,
        )
    })
    .join()
    .unwrap()
    .unwrap();
    assert!(observed.is_none());
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Queued
    );
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    drop(guard);
    assert!(recovery.drain().unwrap());
}

#[cfg(windows)]
#[test]
fn active_probe_session_does_not_turn_queued_job_into_capacity_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let (host, recovery) = crate::ProbeHost::new(1).unwrap();
    let (engine, _) = Engine::open_with_process_hosts(
        EngineConfig {
            data_dir: temp.path().join("data"),
            ..EngineConfig::default()
        },
        None,
        host,
    )
    .unwrap();
    let engine = Arc::new(engine);
    let actor = PrincipalId::new("managed-active-admin").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let session = crate::live_evidence::EvidenceProbeSession::new_with_probe_host(
        &crate::live_evidence::ProbeLimits::default(),
        engine.probe_host.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(recovery.occupied_slots().unwrap(), 1);
    let observed = run_one_queued(
        &engine,
        "active-resource-runner",
        &AtomicBool::new(false),
        true,
    )
    .unwrap();
    assert!(observed.is_none());
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Queued
    );
    drop(session);
    assert!(recovery.drain().unwrap());
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Queued
    );
}

#[cfg(windows)]
#[test]
fn recovered_runner_admission_does_not_remain_poisoned_after_panic() {
    let temp = tempfile::tempdir().unwrap();
    let (host, recovery) = crate::ProbeHost::new(1).unwrap();
    let (engine, _) = Engine::open_with_process_hosts(
        EngineConfig {
            data_dir: temp.path().join("data"),
            ..EngineConfig::default()
        },
        None,
        host,
    )
    .unwrap();
    let engine = Arc::new(engine);
    let other = Arc::clone(&engine);
    let failed = std::thread::spawn(move || {
        let _guard = other.runner_admission.lock().unwrap();
        std::panic::panic_any(String::from("original-admission-panic"));
    })
    .join()
    .unwrap_err();
    assert_eq!(
        failed.downcast_ref::<String>().unwrap(),
        "original-admission-panic"
    );
    assert!(engine.runner_admission.is_poisoned());
    assert!(recovery.drain().unwrap());
    let next =
        run_one_queued(&engine, "after-panic-runner", &AtomicBool::new(false), true).unwrap();
    assert!(
        next.is_none(),
        "empty queue remains empty after taking the original admission lock"
    );
}
