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
