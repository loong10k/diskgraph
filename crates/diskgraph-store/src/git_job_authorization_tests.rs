use crate::git_job_test_fixtures::{batch, fixture};
use crate::{ControlStore, JobKind, JobState, StoreError};
use diskgraph_core::{GitEvidenceJobInput, JobRequestAuthority};

#[test]
fn typed_git_input_and_original_authority_merge_only_when_identical() {
    let (mut control, _, input, authority) = fixture();
    let first = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert_eq!(
        control
            .create_git_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap()
            .job_id,
        first.job_id
    );
    let other = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        input.base_revision_id().into(),
        1,
        input.limits().clone(),
    )
    .unwrap();
    assert_ne!(
        control
            .create_git_evidence_job(&other, &authority, 8)
            .unwrap()
            .unwrap()
            .job_id,
        first.job_id
    );
    let different_exp = JobRequestAuthority::authenticated_remote(
        authority.principal().clone(),
        "issuer",
        "http",
        authority.capabilities().unwrap().to_vec(),
        authority.expires_at_unix_seconds().unwrap() + 1,
    )
    .unwrap();
    assert_ne!(
        control
            .create_git_evidence_job(&input, &different_exp, 8)
            .unwrap()
            .unwrap()
            .job_id,
        first.job_id
    );
    let local = JobRequestAuthority::trusted_local(authority.principal().clone(), "trusted-engine")
        .unwrap();
    let scan = control
        .create_job_with_authority(input.scope_id(), JobKind::Index, &local, 8)
        .unwrap()
        .unwrap();
    let git = control
        .create_git_evidence_job(&input, &local, 8)
        .unwrap()
        .unwrap();
    assert_ne!(scan.job_id, git.job_id);
    assert_eq!(
        control
            .create_job_with_authority(input.scope_id(), JobKind::Sync, &local, 8)
            .unwrap()
            .unwrap()
            .job_id,
        scan.job_id
    );
    assert_eq!(control.git_evidence_job_input(&git.job_id).unwrap(), input);
    assert!(
        control
            .create_job(
                input.scope_id(),
                JobKind::GitEvidence,
                authority.principal()
            )
            .is_err()
    );
    assert!(
        control
            .create_job_with_authority(input.scope_id(), JobKind::GitEvidence, &local, 8)
            .is_err()
    );
}

#[test]
fn every_git_permission_is_required_in_ceiling_and_old_heartbeat_or_fence_apis() {
    for withheld in JobKind::GitEvidence.required_permissions() {
        let (mut control, _, input, authority) = fixture();
        let narrow = JobRequestAuthority::authenticated_remote(
            authority.principal().clone(),
            "issuer",
            "http",
            authority
                .capabilities()
                .unwrap()
                .iter()
                .copied()
                .filter(|p| p != withheld)
                .collect(),
            authority.expires_at_unix_seconds().unwrap(),
        )
        .unwrap();
        assert!(control.create_git_evidence_job(&input, &narrow, 8).is_err());
        assert_eq!(
            control
                .active_job_count_for_principal(authority.principal())
                .unwrap(),
            0
        );
        let job = control
            .create_git_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap();
        let claimed = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
        control
            .connection
            .execute(
                "DELETE FROM grants WHERE permission=?1",
                [withheld.wire_name()],
            )
            .unwrap();
        assert!(
            control
                .heartbeat_fenced(&job.job_id, "owner", claimed.fencing_token)
                .is_err()
        );
        let called = std::cell::Cell::new(false);
        assert!(
            control
                .with_job_authority_fence(&job.job_id, "owner", claimed.fencing_token, &[], || {
                    called.set(true);
                    Ok(())
                })
                .is_err()
        );
        assert!(!called.get());
        assert!(
            control
                .with_job_fence(&job.job_id, "owner", claimed.fencing_token, || {
                    called.set(true);
                    Ok(())
                })
                .is_err()
        );
        assert!(!called.get());
    }
}

#[test]
fn missing_git_input_is_failed_instead_of_falling_back_to_trusted_scan() {
    let (mut control, _, input, authority) = fixture();
    let legacy = control
        .create_job(input.scope_id(), JobKind::Index, authority.principal())
        .unwrap();
    control
        .connection
        .execute(
            "UPDATE jobs SET kind='git_evidence' WHERE job_id=?1",
            [&legacy.job_id],
        )
        .unwrap();
    assert!(matches!(
        control.claim_job_once_strict(&legacy.job_id, "owner"),
        Err(StoreError::InvalidGraph(_))
    ));
    assert_eq!(control.job(&legacy.job_id).unwrap().state, JobState::Failed);
}

#[test]
fn pending_git_protects_its_base_and_survives_reap_for_receipt_reconciliation() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert!(!control.with_retention_guard(input.scope_id(), Ok).unwrap());
    control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    control
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    control.revoke_scope(input.scope_id()).unwrap();
    assert!(
        control
            .cancellation_requested(&job.job_id, control.job(&job.job_id).unwrap().fencing_token)
            .unwrap()
    );
    assert_eq!(control.reap_unclaimable_jobs().unwrap(), 0);
    let candidates = control.list_queued_jobs_limited(64).unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].job_id, job.job_id);
    assert!(
        control
            .claim_job_once_strict(&job.job_id, "new-owner")
            .is_err()
    );
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Cancelled);
}

#[test]
fn committed_receipt_recovers_after_real_expiry_cancel_revoke_and_does_not_steal_a_live_lease() {
    let (mut control, mut graph, input, original) = fixture();
    let expiry = ControlStore::now_ms() / 1000 + 1;
    let authority = JobRequestAuthority::authenticated_remote(
        original.principal().clone(),
        "issuer",
        "http",
        original.capabilities().unwrap().to_vec(),
        expiry,
    )
    .unwrap();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let claim = control
        .claim_job_once_strict(&job.job_id, "publisher")
        .unwrap();
    let receipt = graph
        .publish_git_collector_revision_checked(
            &job.job_id,
            &input,
            (claim.fencing_token, ControlStore::now_ms()),
            "published",
            &batch(&input, "run-one"),
            || Ok(()),
        )
        .unwrap();
    assert!(matches!(
        control.recover_committed_git_job(&job.job_id, "other-owner", &receipt),
        Err(StoreError::StaleOwner)
    ));
    assert_eq!(control.job(&job.job_id).unwrap().owner, "publisher");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while ControlStore::now_ms() / 1000 < expiry {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    control
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    control.request_cancel(&job.job_id).unwrap();
    control.revoke_scope(input.scope_id()).unwrap();
    control.revoke_policy().unwrap();
    assert_eq!(control.reap_unclaimable_jobs().unwrap(), 0);
    let completed = control
        .recover_committed_git_job(&job.job_id, "recovery", &receipt)
        .unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert_eq!(completed.fencing_token, claim.fencing_token + 1);
    assert_eq!(
        control
            .recover_committed_git_job(&job.job_id, "again", &receipt)
            .unwrap(),
        completed
    );
    assert_eq!(
        graph.job_publication_receipt(&job.job_id).unwrap(),
        Some(receipt)
    );
    assert_eq!(
        graph
            .connection
            .query_row("SELECT COUNT(*) FROM collector_runs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn recovery_rejects_a_validly_encoded_receipt_for_a_different_fixed_target() {
    let (mut control, _, input, authority) = fixture();
    let job = control
        .create_git_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let claim = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    control
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    let other = GitEvidenceJobInput::new(
        input.server_id().clone(),
        input.scope_id().clone(),
        "base".into(),
        1,
        input.limits().clone(),
    )
    .unwrap();
    let receipt = diskgraph_core::JobPublicationReceipt::new(
        job.job_id.clone(),
        other,
        "snapshot".into(),
        "published".into(),
        "run".into(),
        (claim.fencing_token, 100),
    )
    .unwrap();
    assert!(matches!(
        control.recover_committed_git_job(&job.job_id, "new-owner", &receipt),
        Err(StoreError::InvalidGraph(_))
    ));
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Running);
}
