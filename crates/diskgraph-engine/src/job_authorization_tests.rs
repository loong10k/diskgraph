//! SC-06 任务授权执行窗口；来源：真实扫描、持久控制库和请求局部同步点。
//! 到期测试使用原始 Unix 秒；不回写认证上下文或替换生产时钟。

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
use crate::EngineConfig;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use diskgraph_core::{JobRequestAuthority, Permission, PrincipalId, ScopeId};
use diskgraph_store::{JobKind, JobState};
use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type ObservationHook = (String, Box<dyn FnOnce()>);
thread_local! { static AFTER_OBSERVE: RefCell<Option<ObservationHook>> = RefCell::new(None); }

pub(super) fn after_observe(job_id: &str) {
    let callback = AFTER_OBSERVE.with(|slot| {
        if slot.borrow().as_ref().is_some_and(|(job, _)| job == job_id) {
            slot.borrow_mut().take().map(|(_, callback)| callback)
        } else {
            None
        }
    });
    if let Some(callback) = callback {
        callback();
    }
}

fn fixture() -> (tempfile::TempDir, Engine, PrincipalId, ScopeId) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='authority'\nversion='0.1.0'\n",
    )
    .unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("authority-execution-fixture").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    (temp, engine, actor, scope)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn authority(actor: &PrincipalId, expiry: u64) -> JobRequestAuthority {
    JobRequestAuthority::authenticated_remote(
        actor.clone(),
        "fixture-issuer",
        "verified-test-adapter",
        vec![Permission::IndexWrite],
        expiry,
    )
    .unwrap()
}

fn wait_until(expiry: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while now() < expiry {
        assert!(
            Instant::now() < deadline,
            "real clock failed to reach the admitted expiry"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_no_publication(temp: &tempfile::TempDir, engine: &Engine, scope: &ScopeId) {
    assert!(engine.latest_revision(scope).unwrap().is_none());
    let db = rusqlite::Connection::open(temp.path().join("data/diskgraph.sqlite")).unwrap();
    for table in [
        "scan_staging",
        "snapshots",
        "graph_revisions",
        "collector_runs",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "failed authority left {table}");
    }
}

#[test]
fn live_remote_authority_can_publish_through_the_strict_runner() {
    let (_temp, engine, actor, scope) = fixture();
    let context = authority(&actor, now() + 300);
    let job = engine
        .index_scope_with_authority(&scope, &context, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert_eq!(
        engine
            .control_store()
            .unwrap()
            .job_request_authority(&job.job_id)
            .unwrap(),
        Some(context)
    );
    let record = engine
        .run_job_strict(&job.job_id, "live-authority")
        .unwrap();
    assert_eq!(record.state, JobState::Completed);
    assert!(engine.latest_revision(&scope).unwrap().is_some());
}

#[test]
fn strict_execution_does_not_upgrade_a_legacy_job_to_local_authority() {
    let (temp, engine, actor, scope) = fixture();
    // 公开可信旧 Store 入口创建缺 authority 的旧式记录；不使用损坏 SQL 夹具。
    let job = engine
        .control_store()
        .unwrap()
        .create_job(&scope, JobKind::Index, &actor)
        .unwrap();
    assert!(
        engine
            .control_store()
            .unwrap()
            .job_request_authority(&job.job_id)
            .unwrap()
            .is_none()
    );
    assert!(engine.run_job_strict(&job.job_id, "strict-legacy").is_err());
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_no_publication(&temp, &engine, &scope);
}

#[test]
fn trusted_runner_still_obeys_a_persisted_remote_expiry() {
    let (temp, engine, actor, scope) = fixture();
    let expiry = now() + 2;
    let job = engine
        .index_scope_with_authority(
            &scope,
            &authority(&actor, expiry),
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert!(now() < expiry, "fixture missed actual admission window");
    wait_until(expiry);
    assert!(
        engine
            .run_job(&job.job_id, "trusted-cannot-upgrade")
            .is_err()
    );
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_no_publication(&temp, &engine, &scope);
}

#[test]
fn remote_expiry_after_native_observation_prevents_staging_and_publication() {
    let (temp, engine, actor, scope) = fixture();
    let expiry = now() + 3;
    let job = engine
        .index_scope_with_authority(
            &scope,
            &authority(&actor, expiry),
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    AFTER_OBSERVE.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                assert!(
                    now() < expiry,
                    "real native observation was not reached before expiry"
                );
                wait_until(expiry);
            }),
        ))
    });
    let result = engine.run_job_strict(&job.job_id, "observed-expiry");
    assert!(
        AFTER_OBSERVE.with(|slot| slot.borrow().is_none()),
        "native observation hook not reached: result={result:?}, expiry={expiry}, now={}",
        now()
    );
    assert!(result.is_err(), "expired observation published: {result:?}");
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_no_publication(&temp, &engine, &scope);
}

#[test]
fn live_grant_revocation_after_native_observation_prevents_publication() {
    let (temp, engine, actor, scope) = fixture();
    let job = engine
        .index_scope_with_authority(
            &scope,
            &authority(&actor, now() + 300),
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let control_path = temp.path().join("data/diskgraph-control.sqlite");
    let revoked_scope = scope.clone();
    AFTER_OBSERVE.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                let mut control = diskgraph_store::ControlStore::open(&control_path).unwrap();
                control
                    .revoke_grant(&actor, &Permission::IndexWrite, &revoked_scope)
                    .unwrap();
            }),
        ))
    });
    let result = engine.run_job_strict(&job.job_id, "observed-revocation");
    assert!(
        AFTER_OBSERVE.with(|slot| slot.borrow().is_none()),
        "native observation hook not reached"
    );
    assert!(result.is_err(), "revoked observation published: {result:?}");
    assert_eq!(
        engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_no_publication(&temp, &engine, &scope);
}

#[test]
fn claimed_job_graph_preparation_wait_consumes_the_original_scan_budget() {
    let (temp, mut engine, actor, scope) = fixture();
    engine.scan_budget.max_duration_ms = 50;
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let engine = std::sync::Arc::new(engine);
    // 真实共享图库锁阻塞旧 staging 清理；不阻塞控制库的持久认领。
    let graph_guard = engine.graph().unwrap();
    let worker_engine = std::sync::Arc::clone(&engine);
    let worker_job = job.job_id.clone();
    let worker = std::thread::spawn(move || worker_engine.run_job(&worker_job, "claim-clock"));
    let admission_limit = Instant::now() + Duration::from_secs(5);
    let mut claimed = false;
    while Instant::now() < admission_limit {
        if engine.job_status(&job.job_id).unwrap().state == JobState::Running {
            claimed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // 计时从确认 Running 后开始，保证耗时属于认领后准备而非队列等待。
    if claimed {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(graph_guard);
    let result = worker.join().unwrap();
    let state = engine.job_status(&job.job_id).unwrap().state;
    let latest = engine.latest_revision(&scope).unwrap();
    eprintln!(
        "claimed={claimed}, original budget=50ms, held after claim=100ms, state={state:?}, latest={latest:?}, result={result:?}"
    );
    assert!(claimed, "fixture did not observe the actual durable claim");
    assert!(
        matches!(
            result,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            )) | Err(crate::EngineError::Store(
                diskgraph_store::StoreError::BudgetExceeded
            ))
        ),
        "claim-to-preparation elapsed time was not charged: {result:?}"
    );
    assert_eq!(state, JobState::Failed);
    assert_no_publication(&temp, &engine, &scope);
}
