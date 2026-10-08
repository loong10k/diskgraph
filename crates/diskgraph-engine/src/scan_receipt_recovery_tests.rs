//! 用真实隔离图库提交模拟结算前崩溃；不把直接扫描建夹具称为原生 worker 验收。
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::PrincipalId;
use diskgraph_store::{JobState, ScanPublicationReceipt, StoreError};

fn fixture() -> (tempfile::TempDir, Engine, ScanPublicationReceipt) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"payload").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("scan-receipt-owner").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let scope = engine.register_scope(&root, &principal, &policy).unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .control()
        .unwrap()
        .claim_job_once_strict(&job.job_id, "old-owner")
        .unwrap();
    let authority = engine
        .control()
        .unwrap()
        .job_request_authority(&job.job_id)
        .unwrap();
    let graph = diskgraph_disktree::scan_native(&root, Default::default()).unwrap();
    let receipt = ScanPublicationReceipt::new(
        &job,
        engine.server_id().unwrap(),
        authority,
        graph.snapshot.id.clone(),
        format!("rev-{}-{}", job.job_id, job.fencing_token),
        1,
    )
    .unwrap();
    let staging = format!("{}:{}", job.job_id, job.fencing_token);
    {
        let mut store = engine.graph().unwrap();
        store.append_staging_nodes(&staging, &graph.nodes).unwrap();
        store
            .publish_scan_revision_checked(&staging, &graph, &receipt, None, || Ok(()))
            .unwrap();
    }
    // 根已消失，任何再次采集都会失败；正确恢复只读取原不可变回执。
    std::fs::remove_dir_all(root).unwrap();
    (dir, engine, receipt)
}

fn expire(dir: &tempfile::TempDir, job: &str) {
    let connection =
        rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [job],
        )
        .unwrap();
}

#[test]
fn scan_receipt_recovery_does_not_preempt_a_live_owner() {
    let (_dir, engine, receipt) = fixture();
    assert!(matches!(
        engine.run_job_strict(receipt.job_id(), "new-owner"),
        Err(EngineError::Store(StoreError::StaleOwner))
    ));
}

#[test]
fn committed_scan_recovers_after_revocation_without_rescanning_and_is_idempotent() {
    let (dir, engine, receipt) = fixture();
    let job = engine.control().unwrap().job(receipt.job_id()).unwrap();
    engine
        .control()
        .unwrap()
        .revoke_scope(&job.scope_id)
        .unwrap();
    expire(&dir, receipt.job_id());
    let result = engine
        .run_job_strict(receipt.job_id(), "new-owner")
        .unwrap();
    assert_eq!(result.state, JobState::Completed);
    assert_eq!(result.fencing_token, receipt.publishing_fence() + 1);
    assert_eq!(
        engine
            .graph()
            .unwrap()
            .scan_publication_receipt(receipt.job_id())
            .unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        engine
            .run_job_strict(receipt.job_id(), "another-owner")
            .unwrap(),
        result
    );
    assert!(
        engine
            .graph()
            .unwrap()
            .revision(receipt.revision_id())
            .is_ok()
    );
}

#[test]
fn queue_reconciles_revoked_committed_scan_before_cancellation_reaping() {
    let (dir, engine, receipt) = fixture();
    let job = engine.control().unwrap().job(receipt.job_id()).unwrap();
    engine
        .control()
        .unwrap()
        .revoke_scope(&job.scope_id)
        .unwrap();
    expire(&dir, receipt.job_id());
    assert!(engine.queued_jobs_limited(64).unwrap().is_empty());
    assert_eq!(
        engine.job_status(receipt.job_id()).unwrap().state,
        JobState::Completed
    );
}

#[test]
fn recovered_job_result_uses_original_receipt_instead_of_incremented_fence() {
    let (dir, engine, receipt) = fixture();
    expire(&dir, receipt.job_id());
    engine.settle_expired_job(receipt.job_id()).unwrap();
    let actor = PrincipalId::new("scan-receipt-owner").unwrap();
    assert_eq!(
        engine
            .revision_for_job(
                receipt.job_id(),
                &actor,
                &engine.policy_authorizer().unwrap()
            )
            .unwrap(),
        receipt.revision_id()
    );
}

#[test]
fn recovery_rejects_changed_job_provenance_without_settling() {
    for mutation in [
        "UPDATE jobs SET principal='different-actor' WHERE job_id=?1",
        "UPDATE jobs SET kind='sync' WHERE job_id=?1",
        "UPDATE jobs SET fencing_token=0 WHERE job_id=?1",
    ] {
        let (dir, engine, receipt) = fixture();
        expire(&dir, receipt.job_id());
        let connection =
            rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
        connection.execute(mutation, [receipt.job_id()]).unwrap();
        let prior = engine.job_status(receipt.job_id()).unwrap();
        assert!(matches!(
            engine.run_job_strict(receipt.job_id(), "new-owner"),
            Err(EngineError::Store(StoreError::InvalidGraph(_)))
        ));
        assert_eq!(engine.job_status(receipt.job_id()).unwrap(), prior);
    }
}
