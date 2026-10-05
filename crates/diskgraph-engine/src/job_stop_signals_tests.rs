//! 请求取消与授权拒绝分离，沿真实认领、SQLite、扫描和 Git 发布边界验收。
//! 来源：原生 Rust Engine / PF-06；不存在的接口只记编译接线缺失，不充行为 RED。

use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use crate::{EngineError, git_evidence_execution_tests};
use diskgraph_core::{BusinessError, Permission};
use diskgraph_store::{JobState, StoreError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn flags() -> (Arc<AtomicBool>, Arc<AtomicBool>) {
    (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
}

#[test]
fn a_request_set_before_claim_is_consumed_by_that_claim_without_publication() {
    let f = GitEvidenceFixture::new();
    let job = f
        .engine
        .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
        .unwrap();
    let (request, denied) = flags();
    request.store(true, Ordering::SeqCst);
    let result =
        f.engine
            .run_job_with_stop_signals(&job.job_id, "pre-claim-request", request, denied);
    assert!(
        result.is_err(),
        "preexisting request allowed work: {result:?}"
    );
    let terminal = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(terminal.state, JobState::Cancelled);
    assert_eq!(terminal.owner, "pre-claim-request");
    assert!(terminal.fencing_token > job.fencing_token);
    assert!(
        f.engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, terminal.fencing_token)
            .unwrap()
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    f.assert_no_git_publication();
}

#[test]
fn request_in_the_claim_before_registration_window_survives_operation_view_revocation() {
    let f = GitEvidenceFixture::new();
    let job = f
        .engine
        .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
        .unwrap();
    let (request, denied) = flags();
    std::thread::scope(|threads| {
        // 同 Engine 真实图锁只阻塞 Index 认领后的准备；恢复 Index 不读取图。
        let graph = f.engine.graph().unwrap();
        let execution = threads.spawn(|| {
            f.engine.run_job_with_stop_signals(
                &job.job_id,
                "claim-gap",
                request.clone(),
                denied.clone(),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let running = loop {
            let running = f.engine.job_status(&job.job_id).unwrap();
            if running.state == JobState::Running {
                break running;
            }
            assert!(
                Instant::now() < deadline,
                "never reached actual Running claim"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
        f.engine
            .control_store()
            .unwrap()
            .revoke_grant(&f.actor, &Permission::OperationView, &f.scope)
            .unwrap();
        let denied_cancel = f.engine.cancel_job(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        );
        assert!(matches!(
            denied_cancel,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
        assert!(
            !f.engine
                .control_store()
                .unwrap()
                .cancellation_requested(&job.job_id, running.fencing_token)
                .unwrap()
        );
        request.store(true, Ordering::SeqCst);
        drop(graph);
        let result = execution.join().unwrap();
        assert!(
            result.is_err(),
            "registration gap lost caller signal: {result:?}"
        );
        let terminal = f.engine.job_status(&job.job_id).unwrap();
        assert_eq!(terminal.state, JobState::Cancelled);
        assert_eq!(terminal.owner, running.owner);
        assert_eq!(terminal.fencing_token, running.fencing_token);
        assert!(
            f.engine
                .control_store()
                .unwrap()
                .cancellation_requested(&job.job_id, running.fencing_token)
                .unwrap()
        );
    });
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    f.assert_no_git_publication();
}

#[test]
fn running_keeper_distinguishes_request_cancel_from_verified_deny_stop() {
    for deny in [false, true] {
        let f = GitEvidenceFixture::new();
        let job = f.enqueue(&f.base, now() + 300);
        let (request, denied) = flags();
        let engine = f.engine.clone();
        let id = job.job_id.clone();
        let actor = f.actor.clone();
        let scope = f.scope.clone();
        let caller_request = request.clone();
        let caller_denied = denied.clone();
        git_evidence_execution_tests::at_publication(&job.job_id, move || {
            let running = engine.job_status(&id).unwrap();
            assert_eq!(running.state, JobState::Running);
            let active = engine.cancellations().unwrap().get(&id).unwrap().clone();
            engine
                .control_store()
                .unwrap()
                .revoke_grant(&actor, &Permission::OperationView, &scope)
                .unwrap();
            assert!(matches!(
                engine.cancel_job(&id, &actor, &engine.policy_authorizer().unwrap()),
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ));
            // 同时到达时，实际授权拒绝优先；不能把它伪写为用户取消。
            caller_denied.store(deny, Ordering::SeqCst);
            caller_request.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !active.load(Ordering::SeqCst) {
                assert!(
                    Instant::now() < deadline,
                    "keeper did not consume original stop signals"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(Arc::ptr_eq(
                engine.cancellations().unwrap().get(&id).unwrap(),
                &active
            ));
            assert_eq!(
                engine
                    .control_store()
                    .unwrap()
                    .cancellation_requested(&id, running.fencing_token)
                    .unwrap(),
                !deny
            );
        });
        let result =
            f.engine
                .run_job_with_stop_signals(&job.job_id, "signal-keeper", request, denied);
        git_evidence_execution_tests::assert_publication_reached(&result);
        if deny {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "verified denial was replaced: {result:?}"
            );
        } else {
            assert!(
                result.is_err(),
                "request cancellation published: {result:?}"
            );
        }
        let terminal = f.engine.job_status(&job.job_id).unwrap();
        assert_eq!(
            terminal.state,
            if deny {
                JobState::Failed
            } else {
                JobState::Cancelled
            }
        );
        assert_eq!(
            f.engine
                .control_store()
                .unwrap()
                .cancellation_requested(&job.job_id, terminal.fencing_token)
                .unwrap(),
            !deny
        );
        assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
        f.assert_no_git_publication();
    }
}

#[test]
fn keeper_authorization_failure_is_not_inferred_from_the_stop_atomic() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    // 仅观察既有真实阶段；捕获前见证不代表采样已成功，权限变更仍只发生在发布前。
    let capture_reached = Arc::new(AtomicBool::new(false));
    let capture_witness = capture_reached.clone();
    git_evidence_execution_tests::at_capture(&job.job_id, move || {
        capture_witness.store(true, Ordering::SeqCst);
    });
    let publication_reached = Arc::new(AtomicBool::new(false));
    let publication_witness = publication_reached.clone();
    let engine = f.engine.clone();
    let id = job.job_id.clone();
    let scope = f.scope.clone();
    let actor = f.actor.clone();
    git_evidence_execution_tests::at_publication(&job.job_id, move || {
        publication_witness.store(true, Ordering::SeqCst);
        let active = engine.cancellations().unwrap().get(&id).unwrap().clone();
        engine
            .control_store()
            .unwrap()
            .revoke_grant(&actor, &Permission::ContentRead, &scope)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !active.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "keeper did not observe real ContentRead loss"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let (request, denied) = flags();
    let call_started = Instant::now();
    let result = f.engine.run_job_with_stop_signals(
        &job.job_id,
        "real-grant-denial",
        request.clone(),
        denied.clone(),
    );
    let call_elapsed = call_started.elapsed();
    // 在阶段断言前保留真实返回及持久诊断；诊断读取失败也打印原 Result，不遮住执行错误。
    let terminal_diagnostic = f.engine.job_status(&job.job_id);
    let failure_diagnostic = f
        .engine
        .control_store()
        .map(|control| control.git_job_failure(&job.job_id));
    let captured = capture_reached.load(Ordering::SeqCst);
    let publication = publication_reached.load(Ordering::SeqCst);
    eprintln!(
        "keeper_boundary_diagnostic: capture_before={captured}, publication_before={publication}, call_elapsed={call_elapsed:?}, result={result:?}, terminal={terminal_diagnostic:?}, persisted_failure={failure_diagnostic:?}"
    );
    assert!(
        captured && publication,
        "required real capture/publication phases not reached: capture_before={captured}, publication_before={publication}, call_elapsed={call_elapsed:?}, result={result:?}, terminal={terminal_diagnostic:?}, persisted_failure={failure_diagnostic:?}"
    );
    git_evidence_execution_tests::assert_publication_reached(&result);
    assert!(
        matches!(&result, Err(EngineError::Store(StoreError::Conflict(message))) if message == "live job authorization withdrawn"),
        "original typed grant failure was replaced: {result:?}"
    );
    let terminal = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(terminal.state, JobState::Failed);
    assert!(!request.load(Ordering::SeqCst));
    assert!(!denied.load(Ordering::SeqCst));
    assert!(
        !f.engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, terminal.fencing_token)
            .unwrap()
    );
    f.assert_no_git_publication();
}

#[test]
fn stop_signals_after_real_completion_do_not_rewrite_the_committed_fact() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let (request, denied) = flags();
    std::thread::scope(|threads| {
        let (at_publication, publication_reached) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let execution = threads.spawn(|| {
            git_evidence_execution_tests::at_publication(&job.job_id, move || {
                at_publication.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(30)).unwrap();
            });
            f.engine.run_job_with_stop_signals(
                &job.job_id,
                "late-signals",
                request.clone(),
                denied.clone(),
            )
        });
        publication_reached
            .recv_timeout(Duration::from_secs(30))
            .unwrap();
        // 已登记的本代 Arc 仍存活；真实 Map 锁只延后末端 RAII 释放，不阻止图提交/持久终态。
        let held = f.engine.cancellations().unwrap();
        assert!(held.contains_key(&job.job_id));
        resume.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let completed = loop {
            let completed = f.engine.job_status(&job.job_id).unwrap();
            if completed.state == JobState::Completed {
                break completed;
            }
            assert!(Instant::now() < deadline, "real publication did not finish");
            std::thread::sleep(Duration::from_millis(2));
        };
        let receipt = f
            .engine
            .revision_reader()
            .unwrap()
            .job_publication_receipt(&job.job_id)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.job_id(), job.job_id);
        request.store(true, Ordering::SeqCst);
        denied.store(true, Ordering::SeqCst);
        drop(held);
        assert_eq!(execution.join().unwrap().unwrap(), completed);
        assert_eq!(f.engine.job_status(&job.job_id).unwrap(), completed);
        assert!(
            !f.engine
                .control_store()
                .unwrap()
                .cancellation_requested(&job.job_id, completed.fencing_token)
                .unwrap()
        );
        assert_eq!(
            f.engine
                .revision_reader()
                .unwrap()
                .job_publication_receipt(&job.job_id)
                .unwrap()
                .unwrap(),
            receipt
        );
    });
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
}

#[test]
fn original_run_job_remains_a_real_compatible_completion_entry() {
    let f = GitEvidenceFixture::new();
    let job = f
        .engine
        .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
        .unwrap();
    assert_eq!(
        f.engine
            .run_job(&job.job_id, "old-compatible")
            .unwrap()
            .state,
        JobState::Completed
    );
}
