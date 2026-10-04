use crate::process_job_test_fixtures::{batch, fixture};
use crate::{ControlStore, JobState, StoreError};
use diskgraph_core::{JobRequestAuthority, Permission};
#[test]
fn receipt_recovery_preserves_committed_fact_after_expiry_and_revocation() {
    let (mut control, mut graph, input, original) = fixture();
    let exp = ControlStore::now_ms() / 1000 + 3;
    let authority = JobRequestAuthority::authenticated_remote(
        original.principal().clone(),
        "issuer",
        "http",
        vec![Permission::MetadataRead, Permission::IndexWrite],
        exp,
    )
    .unwrap();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    let receipt = graph
        .publish_process_collector_revision_checked(
            &job.job_id,
            &input,
            (running.fencing_token, 1001),
            "published",
            &batch(&input, "published", true, false),
            || Ok(()),
        )
        .unwrap();
    assert!(matches!(
        control.recover_committed_process_job(&job.job_id, "new-owner", &receipt),
        Err(StoreError::StaleOwner)
    ));
    control
        .connection
        .execute(
            "DELETE FROM grants WHERE permission=?1",
            [Permission::MetadataRead.wire_name()],
        )
        .unwrap();
    control
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    while ControlStore::now_ms() / 1000 < exp {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let finished = control
        .recover_committed_process_job(
            &job.job_id,
            "new-owner",
            &graph
                .process_job_publication_receipt(&job.job_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(finished.state, JobState::Completed);
    assert_eq!(finished.fencing_token, running.fencing_token + 1);
    assert_eq!(
        control.job_request_authority(&job.job_id).unwrap(),
        Some(authority)
    );
    assert_eq!(
        graph.process_job_publication_receipt(&job.job_id).unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        control
            .recover_committed_process_job(&job.job_id, "another-owner", &receipt)
            .unwrap(),
        finished
    );
    assert_eq!(
        crate::process_job_test_fixtures::count(&graph, "collector_runs"),
        1
    );
}
#[test]
fn pending_process_job_protects_history_and_missing_receipt_never_completes() {
    let (mut control, graph, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert!(!control.with_retention_guard(input.scope_id(), Ok).unwrap());
    assert_eq!(
        graph.process_job_publication_receipt(&job.job_id).unwrap(),
        None
    );
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Queued);
    control.cancel_queued(&job.job_id).unwrap();
    assert!(control.with_retention_guard(input.scope_id(), Ok).unwrap());
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Cancelled);
}

#[test]
fn pruned_process_result_keeps_its_unique_committed_receipt() {
    let (mut control, mut graph, input, authority) = fixture();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    let running = control.claim_job_once_strict(&job.job_id, "owner").unwrap();
    let receipt = graph
        .publish_process_collector_revision_checked(
            &job.job_id,
            &input,
            (running.fencing_token, 1001),
            "published",
            &batch(&input, "published", true, false),
            || Ok(()),
        )
        .unwrap();
    control
        .finish_process_job_fenced(
            &job.job_id,
            "owner",
            running.fencing_token,
            JobState::Completed,
            None,
        )
        .unwrap();
    let mut newer = crate::tests::graph("new-snapshot", 2000);
    newer.evidence.clear();
    graph
        .append_staging_nodes("new-stage", &newer.nodes)
        .unwrap();
    graph
        .publish_revision_owned(
            "new-stage",
            &newer,
            "new-base",
            2000,
            Some((input.server_id().as_str(), input.scope_id().as_str())),
        )
        .unwrap();
    let pruned = graph
        .prune_revisions(
            input.server_id().as_str(),
            input.scope_id().as_str(),
            1,
            true,
        )
        .unwrap();
    assert!(
        pruned
            .iter()
            .any(|revision| revision.revision_id == "published")
    );
    assert!(matches!(
        graph.revision("published"),
        Err(StoreError::RevisionNotFound(_))
    ));
    assert_eq!(
        graph.process_job_publication_receipt(&job.job_id).unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        control
            .recover_committed_process_job(&job.job_id, "reconnect", &receipt)
            .unwrap()
            .state,
        JobState::Completed
    );
    assert_eq!(
        crate::process_job_test_fixtures::count(&graph, "process_job_publication_receipts"),
        1
    );
    assert_eq!(
        crate::process_job_test_fixtures::count(&graph, "collector_runs"),
        0
    );
}
