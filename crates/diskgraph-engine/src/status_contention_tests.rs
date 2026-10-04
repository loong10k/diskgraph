//! 状态查询控制锁竞争回归；来源：实际 Engine 单一控制库与请求期限。
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use std::time::{Duration, Instant};

#[test]
fn status_contention_waits_for_short_owner_without_false_budget_failure() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let policy = f.engine.policy_authorizer().unwrap();
    let guard = f.engine.control_store().unwrap();
    std::thread::scope(|threads| {
        let (started, ready) = std::sync::mpsc::channel();
        let f = &f;
        let job = &job;
        let policy = &policy;
        let query = threads.spawn(move || {
            started.send(()).unwrap();
            f.engine
                .git_job_status_details(&job.job_id, &f.actor, policy)
        });
        ready.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        drop(guard);
        let value = query
            .join()
            .unwrap()
            .expect("live request must survive short control contention")
            .unwrap();
        assert_eq!(value["job_id"], job.job_id);
        assert_eq!(value["state"], "queued");
    });
}

#[test]
fn status_contention_does_not_wait_past_original_deadline() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let policy = f.engine.policy_authorizer().unwrap();
    let guard = f.engine.control_store().unwrap();
    std::thread::scope(|threads| {
        let query = threads.spawn(|| {
            let started = Instant::now();
            let result = f
                .engine
                .git_job_status_details(&job.job_id, &f.actor, &policy);
            (result, started.elapsed())
        });
        let (result, elapsed) = query.join().unwrap();
        assert!(matches!(
            result,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            ))
        ));
        assert!(
            elapsed < Duration::from_secs(2),
            "lock owner still alive: {elapsed:?}"
        );
        drop(guard);
    });
}

#[test]
fn expired_status_classification_never_restarts_its_request_window() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let policy = f.engine.policy_authorizer().unwrap();
    let deadline = Instant::now();
    for result in [
        f.engine
            .git_job_status_details_until(&job.job_id, &f.actor, &policy, deadline),
        f.engine
            .process_job_status_details_until(&job.job_id, &f.actor, &policy, deadline),
    ] {
        assert!(matches!(
            result,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::BudgetExceeded
            ))
        ));
    }
}

#[test]
fn status_waiting_for_owner_still_observes_permission_revocation() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let policy = f.engine.policy_authorizer().unwrap();
    let mut guard = f.engine.control_store().unwrap();
    std::thread::scope(|threads| {
        let (started, ready) = std::sync::mpsc::channel();
        let f = &f;
        let job = &job;
        let policy = &policy;
        let query = threads.spawn(move || {
            started.send(()).unwrap();
            f.engine
                .git_job_status_details(&job.job_id, &f.actor, policy)
        });
        ready.recv().unwrap();
        guard
            .revoke_grant(
                &f.actor,
                &diskgraph_core::Permission::OperationView,
                &f.scope,
            )
            .unwrap();
        drop(guard);
        assert!(matches!(
            query.join().unwrap(),
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::PermissionDenied
            ))
        ));
    });
}
