//! 两个真实 Engine 共用持久队列时的本机取消句柄生命周期；不使用生产 hook。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
use crate::EngineConfig;
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use diskgraph_store::{JobKind, JobState};
use std::time::{Duration, Instant};

#[test]
fn enqueue_cannot_register_a_terminal_handle_after_another_engine_completes() {
    let f = GitEvidenceFixture::new();
    let worker = Engine::open(EngineConfig {
        data_dir: f.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let job = std::thread::scope(|threads| {
        // 真实唯一 Map 锁只阻塞 producer 的 commit 后登记，不阻塞另一个 Engine 的持久任务。
        // guard 是 scope closure 的局部值，任何断言 panic 会先解锁，再 join producer。
        let held = f.engine.cancellations().unwrap();
        let producer = threads.spawn(|| f.enqueue(&f.base, now() + 300));
        let deadline = Instant::now() + Duration::from_secs(10);
        let committed = loop {
            if let Some(job) = worker
                .queued_jobs()
                .unwrap()
                .into_iter()
                .find(|job| job.kind == JobKind::GitEvidence)
            {
                break job;
            }
            assert!(
                Instant::now() < deadline,
                "producer did not durably enqueue before map registration"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(committed.state, JobState::Queued);
        assert!(!held.contains_key(&committed.job_id));
        let completed = worker
            .run_job_strict(&committed.job_id, "second-engine-owner")
            .unwrap();
        assert_eq!(completed.state, JobState::Completed);
        let receipt = worker
            .revision_reader()
            .unwrap()
            .job_publication_receipt(&committed.job_id)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.base_revision_id(), f.base);
        drop(held);
        let returned = producer.join().unwrap();
        assert_eq!(returned.job_id, committed.job_id);
        completed
    });
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Completed
    );
    assert!(
        !f.engine.cancellations().unwrap().contains_key(&job.job_id),
        "late producer registration left a terminal handle with no local executor"
    );
}

#[test]
fn queued_producer_handle_does_not_survive_remote_engine_completion() {
    let f = GitEvidenceFixture::new();
    let queued = f.enqueue(&f.base, now() + 300);
    assert_eq!(queued.state, JobState::Queued);
    let worker = Engine::open(EngineConfig {
        data_dir: f.temp.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let completed = worker
        .run_job_strict(&queued.job_id, "second-engine-owner")
        .unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert!(
        worker
            .revision_reader()
            .unwrap()
            .job_publication_receipt(&queued.job_id)
            .unwrap()
            .is_some()
    );
    assert!(
        !f.engine
            .cancellations()
            .unwrap()
            .contains_key(&queued.job_id),
        "producer cached queued handle after another Engine completed the job"
    );
}

#[test]
fn durable_cancel_between_claim_and_local_flag_registration_prevents_publication() {
    let f = GitEvidenceFixture::new();
    let queued = f
        .engine
        .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
        .unwrap();
    assert_eq!(queued.kind, JobKind::Index);
    std::thread::scope(|threads| {
        // Index 对账不读图库；同 Engine 的真实图锁确定阻塞认领后的 staging 准备。
        // guard 在 scope closure 内创建，断言 panic 时先释放图锁，再 join 执行线程。
        let graph_guard = f.engine.graph().unwrap();
        let execution = threads.spawn(|| f.engine.run_job_strict(&queued.job_id, "window-owner"));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if f.engine.job_status(&queued.job_id).unwrap().state == JobState::Running {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "job did not reach the claim-before-registration window"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            !f.engine
                .cancellations()
                .unwrap()
                .contains_key(&queued.job_id)
        );
        f.engine
            .cancel_job(
                &queued.job_id,
                &f.actor,
                &f.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let running = f.engine.job_status(&queued.job_id).unwrap();
        assert_eq!(running.state, JobState::Running);
        assert!(
            f.engine
                .control()
                .unwrap()
                .cancellation_requested(&queued.job_id, running.fencing_token)
                .unwrap()
        );
        drop(graph_guard);
        let result = execution.join().unwrap();
        // ScanObservationGuard 首检读取持久取消，沿原扫描契约返回 Business Conflict。
        assert!(
            matches!(
                result,
                Err(crate::EngineError::Business(
                    diskgraph_core::BusinessError::Conflict
                ))
            ),
            "durable cancellation before registration returned {result:?}"
        );
    });
    assert_eq!(
        f.engine.job_status(&queued.job_id).unwrap().state,
        JobState::Cancelled
    );
    assert!(
        !f.engine
            .cancellations()
            .unwrap()
            .contains_key(&queued.job_id)
    );
    f.assert_no_git_publication();
    let db = rusqlite::Connection::open(f.temp.path().join("data/diskgraph.sqlite")).unwrap();
    for (table, expected) in [
        ("scan_staging", 0),
        ("snapshots", 1),
        ("graph_revisions", 1),
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, expected, "cancelled scan changed {table}");
    }
}
