//! 排队终态与活动代次的取消句柄生命周期；来源：真实 Engine/Control 公共任务接口。
use crate::EngineError;
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use diskgraph_core::{GitEvidenceFailureCode, GitEvidenceFailurePhase, Permission};
use diskgraph_store::{JobState, StoreError};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[test]
fn queued_git_explicit_cancel_releases_the_terminal_handle() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    assert_eq!(job.state, JobState::Queued);
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    f.engine
        .cancel_job(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Cancelled
    );
    let diagnostic = f
        .engine
        .git_job_failure(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.phase(), GitEvidenceFailurePhase::Admission);
    assert_eq!(diagnostic.code(), GitEvidenceFailureCode::Cancelled);
    f.assert_no_git_publication();
    assert!(
        !f.engine.cancellations().unwrap().contains_key(&job.job_id),
        "public queued cancellation must release its terminal handle"
    );
}

#[test]
fn queued_git_live_permission_rejection_releases_the_terminal_handle() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    f.engine
        .control()
        .unwrap()
        .revoke_grant(&f.actor, &Permission::ContentRead, &f.scope)
        .unwrap();
    let result = f.engine.run_job_strict(&job.job_id, "denied-owner");
    assert!(matches!(
        result,
        Err(EngineError::Store(StoreError::Conflict(_)))
    ));
    let terminal = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(terminal.state, JobState::Failed);
    assert_eq!(terminal.fencing_token, 0);
    let diagnostic = f
        .engine
        .git_job_failure(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.phase(), GitEvidenceFailurePhase::Admission);
    assert_eq!(diagnostic.code(), GitEvidenceFailureCode::Conflict);
    f.assert_no_git_publication();
    assert!(
        !f.engine.cancellations().unwrap().contains_key(&job.job_id),
        "failed admission must release the original queued handle"
    );
}

#[test]
fn live_git_owner_rejection_and_cancel_preserve_the_active_handle() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let engine = f.engine.clone();
    let actor = f.actor.clone();
    let id = job.job_id.clone();
    crate::git_evidence_execution_tests::at_publication(&job.job_id, move || {
        // 真正本机执行已完成 capture 并到达发布前；不靠手工 claim 伪造 active Map。
        let active = engine.cancellations().unwrap().get(&id).unwrap().clone();
        let claimed = engine.job_status(&id).unwrap();
        assert_eq!(claimed.state, JobState::Running);
        let result = engine.run_job_strict(&id, "other-owner");
        assert!(matches!(
            result,
            Err(EngineError::Store(StoreError::StaleOwner))
        ));
        assert_eq!(engine.job_status(&id).unwrap(), claimed);
        assert!(Arc::ptr_eq(
            engine.cancellations().unwrap().get(&id).unwrap(),
            &active
        ));
        engine
            .cancel_job(&id, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        assert_eq!(engine.job_status(&id).unwrap().state, JobState::Running);
        assert!(active.load(Ordering::SeqCst));
        assert!(
            Arc::ptr_eq(engine.cancellations().unwrap().get(&id).unwrap(), &active),
            "cancel intent cannot delete the live owner's handle"
        );
    });
    let result = f.engine.run_job_strict(&job.job_id, "active-owner");
    assert!(
        matches!(
            &result,
            Err(EngineError::Store(StoreError::Conflict(message)))
                if message == "job cancellation requested"
        ),
        "publication heartbeat must report the actual durable cancellation: {result:?}"
    );
    crate::git_evidence_execution_tests::assert_publication_reached(&result);
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Cancelled
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    let diagnostic = f
        .engine
        .git_job_failure(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic.phase(), GitEvidenceFailurePhase::Execution);
    assert_eq!(diagnostic.code(), GitEvidenceFailureCode::Cancelled);
    f.assert_no_git_publication();
}

#[test]
fn legacy_scan_terminal_cancel_and_claim_refusal_release_only_queued_handles() {
    for rejected in [false, true] {
        let f = GitEvidenceFixture::new();
        let job = f
            .engine
            .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
            .unwrap();
        assert_eq!(job.state, JobState::Queued);
        assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
        let expected = if rejected {
            f.engine
                .control()
                .unwrap()
                .revoke_grant(&f.actor, &Permission::IndexWrite, &f.scope)
                .unwrap();
            assert!(matches!(
                f.engine.run_job_strict(&job.job_id, "denied-scan"),
                Err(EngineError::Store(StoreError::Conflict(_)))
            ));
            JobState::Failed
        } else {
            f.engine
                .cancel_job(
                    &job.job_id,
                    &f.actor,
                    &f.engine.policy_authorizer().unwrap(),
                )
                .unwrap();
            JobState::Cancelled
        };
        let terminal = f.engine.job_status(&job.job_id).unwrap();
        assert_eq!(terminal.state, expected);
        assert_eq!(terminal.fencing_token, 0);
        assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
        assert_eq!(
            f.engine.latest_revision(&f.scope).unwrap().as_deref(),
            Some(f.base.as_str())
        );
    }
}
