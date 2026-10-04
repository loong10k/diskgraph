use crate::git_job_test_fixtures::fixture;
use crate::{ControlStore, JobState, StoreError};
use diskgraph_core::{GitEvidenceFailure, GitEvidenceFailureCode, GitEvidenceFailurePhase};

fn diagnostic(code: GitEvidenceFailureCode) -> GitEvidenceFailure {
    GitEvidenceFailure::new(GitEvidenceFailurePhase::Execution, code)
}

#[test]
fn failed_git_job_and_safe_diagnostic_reconnect_together_without_raw_error_text() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let claim = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    let failure = diagnostic(GitEvidenceFailureCode::Unsupported);
    let actual = control
        .finish_git_job_fenced(
            &job.job_id,
            "owner",
            claim.fencing_token,
            JobState::Failed,
            Some(&failure),
        )
        .unwrap();
    assert_eq!(actual.state, JobState::Failed);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    control
        .connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .unwrap();
    let reopened = ControlStore::open(&path).unwrap();
    assert_eq!(reopened.job(&job.job_id).unwrap(), actual);
    let json =
        serde_json::to_value(reopened.git_job_failure(&job.job_id).unwrap().unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"phase":"execution","code":"unsupported"})
    );
    assert!(
        reopened
            .connection
            .execute("UPDATE git_job_failures SET code='unavailable'", [])
            .is_err()
    );
    assert!(
        reopened
            .connection
            .execute("DELETE FROM git_job_failures", [])
            .is_err()
    );
}

#[test]
fn invalidated_owner_cannot_write_the_new_owner_terminal_or_diagnostic() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let old = control
        .claim_job_once_strict(&job.job_id, "old-owner")
        .unwrap();
    control
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    let fresh = control
        .claim_job_once_strict(&job.job_id, "new-owner")
        .unwrap();
    assert!(matches!(
        control.finish_git_job_fenced(
            &job.job_id,
            "old-owner",
            old.fencing_token,
            JobState::Failed,
            Some(&diagnostic(GitEvidenceFailureCode::Conflict))
        ),
        Err(StoreError::StaleOwner)
    ));
    assert_eq!(control.git_job_failure(&job.job_id).unwrap(), None);
    assert_eq!(control.job(&job.job_id).unwrap(), fresh);
    control.request_cancel(&job.job_id).unwrap();
    assert_eq!(
        control
            .finish_git_job_fenced(
                &job.job_id,
                "new-owner",
                fresh.fencing_token,
                JobState::Cancelled,
                Some(&diagnostic(GitEvidenceFailureCode::Cancelled))
            )
            .unwrap()
            .state,
        JobState::Cancelled
    );
}

#[test]
fn diagnostic_insert_failure_rolls_back_the_actual_terminal_update() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let claim = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    control.connection.execute_batch("CREATE TRIGGER deny_git_failure BEFORE INSERT ON git_job_failures BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
    assert!(
        control
            .finish_git_job_fenced(
                &job.job_id,
                "owner",
                claim.fencing_token,
                JobState::Failed,
                Some(&diagnostic(GitEvidenceFailureCode::Unavailable))
            )
            .is_err()
    );
    assert_eq!(control.job(&job.job_id).unwrap(), claim);
    assert_eq!(control.git_job_failure(&job.job_id).unwrap(), None);
    assert!(
        control
            .finish_git_job_fenced(
                &job.job_id,
                "owner",
                claim.fencing_token,
                JobState::Failed,
                None
            )
            .is_err()
    );
    assert!(
        control
            .finish_git_job_fenced(
                &job.job_id,
                "owner",
                claim.fencing_token,
                JobState::Cancelled,
                Some(&diagnostic(GitEvidenceFailureCode::Conflict))
            )
            .is_err()
    );
}

#[test]
fn queued_live_grant_failure_is_durable_admission_conflict_not_guessed_owner_loss() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert_eq!(
        control
            .connection
            .execute(
                "DELETE FROM grants WHERE permission=?1",
                [diskgraph_core::Permission::ContentRead.wire_name()]
            )
            .unwrap(),
        1
    );
    assert!(control.claim_job_once_strict(&job.job_id, "owner").is_err());
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Failed);
    assert_eq!(
        control.git_job_failure(&job.job_id).unwrap(),
        Some(GitEvidenceFailure::new(
            GitEvidenceFailurePhase::Admission,
            GitEvidenceFailureCode::Conflict
        ))
    );
}

#[test]
fn queued_explicit_git_cancel_has_a_durable_fixed_admission_diagnostic() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert!(control.request_cancel(&job.job_id).unwrap());
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Cancelled);
    assert_eq!(
        control.git_job_failure(&job.job_id).unwrap(),
        Some(GitEvidenceFailure::new(
            GitEvidenceFailurePhase::Admission,
            GitEvidenceFailureCode::Cancelled
        ))
    );
}

#[test]
fn queued_git_scope_revocation_has_a_durable_fixed_admission_diagnostic() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    control.revoke_scope(input.scope_id()).unwrap();
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Cancelled);
    assert_eq!(
        control.git_job_failure(&job.job_id).unwrap(),
        Some(GitEvidenceFailure::new(
            GitEvidenceFailurePhase::Admission,
            GitEvidenceFailureCode::Cancelled
        ))
    );
}

#[test]
fn queued_git_cancel_compatibility_api_also_has_a_fixed_admission_diagnostic() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert!(control.cancel_queued(&job.job_id).unwrap());
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Cancelled);
    assert_eq!(
        control.git_job_failure(&job.job_id).unwrap(),
        Some(GitEvidenceFailure::new(
            GitEvidenceFailurePhase::Admission,
            GitEvidenceFailureCode::Cancelled
        ))
    );
}

#[test]
fn queued_cancellation_diagnostic_failure_rolls_back_all_three_public_paths() {
    for path in 0..3 {
        let (mut control, _, input, authority) = fixture();
        let job = control
            .create_git_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap();
        control.connection.execute_batch("CREATE TRIGGER deny_queued_diagnostic BEFORE INSERT ON git_job_failures BEGIN SELECT RAISE(ABORT,'fixture refusal'); END;").unwrap();
        let result = match path {
            0 => control.request_cancel(&job.job_id),
            1 => control.cancel_queued(&job.job_id),
            _ => control.revoke_scope(input.scope_id()).map(|()| true),
        };
        assert!(result.is_err(), "public cancellation path {path}");
        assert_eq!(control.job(&job.job_id).unwrap(), job);
        assert!(!control.scope(input.scope_id()).unwrap().revoked);
        assert!(
            !control
                .cancellation_requested(&job.job_id, job.fencing_token)
                .unwrap()
        );
        assert_eq!(control.git_job_failure(&job.job_id).unwrap(), None);
    }
}
