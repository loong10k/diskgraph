use crate::JobState;
use crate::git_job_test_fixtures::fixture;

#[test]
fn legacy_terminal_api_cannot_bypass_typed_git_failure_or_receipt_protocol() {
    for state in [JobState::Failed, JobState::Cancelled, JobState::Completed] {
        let (mut control, _, input, authority) = fixture();
        let job = control
            .create_git_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap();
        let claim = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
        assert!(
            control
                .finish_job_fenced(&job.job_id, "owner", claim.fencing_token, state)
                .is_err(),
            "legacy fenced API accepted Git {state:?}"
        );
        assert_eq!(control.job(&job.job_id).unwrap(), claim);
        assert!(
            control.finish_job(&job.job_id, "owner", state).is_err(),
            "legacy API accepted Git {state:?}"
        );
        assert_eq!(control.job(&job.job_id).unwrap(), claim);
        assert_eq!(control.git_job_failure(&job.job_id).unwrap(), None);
    }
}
