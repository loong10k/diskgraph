use crate::process_job_test_fixtures::fixture;
use crate::{JobKind, JobState, StoreError};
use diskgraph_core::{
    JobRequestAuthority, Permission, ProcessEvidenceFailure, ProcessEvidenceFailureCode,
    ProcessEvidenceFailurePhase,
};
#[test]
fn metadata_only_authority_roundtrips_merges_and_never_uses_old_scan_entry() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert_eq!(job.kind, JobKind::ProcessEvidence);
    assert_eq!(
        control.process_evidence_job_input(&job.job_id).unwrap(),
        input
    );
    assert_eq!(
        control.job_request_authority(&job.job_id).unwrap(),
        Some(authority.clone())
    );
    assert_eq!(
        control
            .create_process_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap()
            .job_id,
        job.job_id
    );
    assert!(matches!(
        control.create_job(
            input.scope_id(),
            JobKind::ProcessEvidence,
            authority.principal()
        ),
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        control.create_job_with_authority(
            input.scope_id(),
            JobKind::ProcessEvidence,
            &authority,
            8
        ),
        Err(StoreError::Conflict(_))
    ));
    let scan = control
        .create_job_with_authority(input.scope_id(), JobKind::Index, &authority, 8)
        .unwrap()
        .unwrap();
    assert_ne!(scan.job_id, job.job_id);
    let other = crate::process_job_test_fixtures::next(&input, "other-base");
    assert_ne!(
        control
            .create_process_evidence_job(&other, &authority, 8)
            .unwrap()
            .unwrap()
            .job_id,
        job.job_id
    );
}
#[test]
fn ceiling_and_live_metadata_are_required_at_claim_and_fence() {
    let (mut control, _, input, authority) = fixture();
    let narrow = JobRequestAuthority::authenticated_remote(
        authority.principal().clone(),
        "issuer",
        "http",
        vec![Permission::IndexWrite],
        crate::ControlStore::now_ms() / 1000 + 60,
    )
    .unwrap();
    assert!(matches!(
        control.create_process_evidence_job(&input, &narrow, 8),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        control
            .active_job_count_for_principal(authority.principal())
            .unwrap(),
        0
    );
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    assert_eq!(
        control
            .connection
            .execute(
                "DELETE FROM grants WHERE permission=?1",
                [Permission::MetadataRead.wire_name()]
            )
            .unwrap(),
        1
    );
    let mut called = false;
    assert!(matches!(
        control.with_job_fence(&job.job_id, "owner", running.fencing_token, || {
            called = true;
            Ok(())
        }),
        Err(StoreError::Conflict(_))
    ));
    assert!(!called);
    assert!(matches!(
        control.heartbeat_fenced(&job.job_id, "owner", running.fencing_token),
        Err(StoreError::Conflict(_))
    ));
    let failure = ProcessEvidenceFailure::new(
        ProcessEvidenceFailurePhase::Execution,
        ProcessEvidenceFailureCode::Conflict,
    );
    assert_eq!(
        control
            .finish_process_job_fenced(
                &job.job_id,
                "owner",
                running.fencing_token,
                JobState::Failed,
                Some(&failure)
            )
            .unwrap()
            .state,
        JobState::Failed
    );
    assert_eq!(
        control.process_job_failure(&job.job_id).unwrap(),
        Some(failure)
    );
}
#[test]
fn queued_scope_or_explicit_cancel_persists_fixed_diagnostic() {
    for scope_revoke in [false, true] {
        let (mut control, _, input, authority) = fixture();
        let job = control
            .create_process_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap();
        if scope_revoke {
            control.revoke_scope(input.scope_id()).unwrap();
        } else {
            control.request_cancel(&job.job_id).unwrap();
        }
        assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Cancelled);
        assert_eq!(
            control.process_job_failure(&job.job_id).unwrap(),
            Some(ProcessEvidenceFailure::new(
                ProcessEvidenceFailurePhase::Admission,
                ProcessEvidenceFailureCode::Cancelled
            ))
        );
    }
}
#[test]
fn obsolete_terminal_entry_and_stale_owner_cannot_settle_process() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    assert!(matches!(
        control.finish_job_fenced(
            &job.job_id,
            "owner",
            running.fencing_token,
            JobState::Failed
        ),
        Err(StoreError::Conflict(_))
    ));
    let failure = ProcessEvidenceFailure::new(
        ProcessEvidenceFailurePhase::Execution,
        ProcessEvidenceFailureCode::Unavailable,
    );
    assert!(matches!(
        control.finish_process_job_fenced(
            &job.job_id,
            "other",
            running.fencing_token,
            JobState::Failed,
            Some(&failure)
        ),
        Err(StoreError::StaleOwner)
    ));
    assert_eq!(control.process_job_failure(&job.job_id).unwrap(), None);
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Running);
}
#[test]
fn missing_typed_input_fails_closed_without_stealing_a_live_lease() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    control
        .connection
        .execute_batch("DROP TRIGGER process_job_input_immutable_delete;")
        .unwrap();
    assert_eq!(
        control
            .connection
            .execute(
                "DELETE FROM process_evidence_job_inputs WHERE job_id=?1",
                [&job.job_id]
            )
            .unwrap(),
        1
    );
    assert!(matches!(
        control.claim_job_once_strict(&job.job_id, "other"),
        Err(StoreError::StaleOwner)
    ));
    assert_eq!(
        control.job(&job.job_id).unwrap().fencing_token,
        running.fencing_token
    );
    control
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    assert!(matches!(
        control.claim_job_once_strict(&job.job_id, "other"),
        Err(StoreError::InvalidGraph(_))
    ));
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Failed);
    assert_eq!(
        control
            .process_job_failure(&job.job_id)
            .unwrap()
            .unwrap()
            .code(),
        ProcessEvidenceFailureCode::InternalError
    );
}

#[test]
fn legacy_reaper_leaves_process_receipt_reconciliation_to_the_typed_runner() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert_eq!(
        control
            .connection
            .execute(
                "DELETE FROM grants WHERE permission=?1",
                [Permission::MetadataRead.wire_name()]
            )
            .unwrap(),
        1
    );
    assert_eq!(control.reap_request_jobs_strict().unwrap(), 0);
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Queued);
    assert_eq!(control.process_job_failure(&job.job_id).unwrap(), None);
    assert!(matches!(
        control.claim_job_once_strict(&job.job_id, "owner"),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Failed);
    assert_eq!(
        control
            .process_job_failure(&job.job_id)
            .unwrap()
            .unwrap()
            .code(),
        ProcessEvidenceFailureCode::Conflict
    );
}
