//! 成功 Git 发布与图已提交事实恢复；来源：真实任务、Git 观察和不可变 publication receipt。
use crate::git_evidence_fixture::{GitEvidenceFixture, now, wait_until};
use diskgraph_core::GitEvidenceSummary;
use diskgraph_store::JobState;

#[test]
fn git_job_publishes_safe_summary_on_same_snapshot_and_preserves_old_revision() {
    let f = GitEvidenceFixture::new();
    std::fs::write(f.temp.path().join("repo/tracked"), "private dirty secret\n").unwrap();
    let head = String::from_utf8(f.git(&["rev-parse", "HEAD"])).unwrap();
    let job = f.enqueue(&f.base, now() + 300);
    assert_eq!(
        f.engine
            .run_job_strict(&job.job_id, "git-owner")
            .unwrap()
            .state,
        JobState::Completed
    );
    let first = f
        .engine
        .revision_for_job(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let reader = f.engine.revision_reader().unwrap();
    let receipt = reader
        .job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.revision_id(), first);
    assert_eq!(
        reader.revision(&first).unwrap().snapshot_id,
        reader.revision(&f.base).unwrap().snapshot_id
    );
    let edge = reader
        .revision_evidence(&first)
        .unwrap()
        .all_edges()
        .unwrap()
        .into_iter()
        .find(|edge| edge.edge_id == format!("{}-ownership", receipt.run_id()))
        .unwrap();
    let evidence = reader
        .revision_evidence(&first)
        .unwrap()
        .evidence_for_edges(&[edge.edge_id])
        .unwrap();
    let value = evidence
        .into_iter()
        .find(|e| e.run_id == receipt.run_id())
        .unwrap();
    let summary: GitEvidenceSummary = serde_json::from_str(&value.basis).unwrap();
    assert_eq!(summary.dirty_count(), 1);
    assert!(!summary.remote_state_known());
    assert_eq!(summary.ahead_of_upstream(), None);
    assert!(value.expires_at_unix_ms.unwrap() > value.observed_at_unix_ms);
    assert!(!value.basis.contains(head.trim()));
    assert!(!value.basis.contains("private dirty secret"));
    assert_eq!(
        value.input_fingerprint,
        summary.observation_fingerprint(receipt.input())
    );
    assert_ne!(
        value.input_fingerprint,
        receipt.input_sha256(),
        "request identity is not the observation fingerprint"
    );
    assert!(
        reader
            .revision_evidence(&f.base)
            .unwrap()
            .all_edges()
            .unwrap()
            .iter()
            .all(|e| !e.edge_id.starts_with(receipt.run_id()))
    );
    drop(reader);
    let second_job = f.enqueue(&first, now() + 300);
    f.engine
        .run_job_strict(&second_job.job_id, "git-second")
        .unwrap();
    let second = f
        .engine
        .revision_for_job(
            &second_job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let reader = f.engine.revision_reader().unwrap();
    assert!(
        reader
            .revision_evidence(&second)
            .unwrap()
            .all_edges()
            .unwrap()
            .iter()
            .all(|e| !e.edge_id.starts_with(receipt.run_id()))
    );
    assert!(
        reader
            .revision_evidence(&first)
            .unwrap()
            .all_edges()
            .unwrap()
            .iter()
            .any(|e| e.edge_id.starts_with(receipt.run_id()))
    );
}

#[test]
fn committed_git_receipt_recovers_after_expiry_without_resampling_or_new_revision() {
    let f = GitEvidenceFixture::new();
    let expiry = now() + 3;
    let job = f.enqueue(&f.base, expiry);
    crate::git_evidence_execution_tests::crash_after_commit(&job.job_id);
    assert!(
        f.engine
            .run_job_strict(&job.job_id, "crashed-owner")
            .is_err()
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Running
    );
    let receipt = f
        .engine
        .revision_reader()
        .unwrap()
        .job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    // 明确模拟进程崩溃后的租约流逝，只推进租约，不伪造输入/authority/receipt。
    rusqlite::Connection::open(f.temp.path().join("data/diskgraph-control.sqlite"))
        .unwrap()
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    wait_until(expiry);
    f.engine
        .set_content_read(&f.scope, &f.actor, false)
        .unwrap();
    std::fs::remove_dir_all(f.temp.path().join("repo")).unwrap();
    let recovered = f
        .engine
        .run_job_strict(&job.job_id, "recover-owner")
        .unwrap();
    assert_eq!(recovered.state, JobState::Completed);
    assert!(recovered.fencing_token > receipt.publishing_fence());
    assert_eq!(
        f.engine
            .revision_for_job(
                &job.job_id,
                &f.actor,
                &f.engine.policy_authorizer().unwrap()
            )
            .unwrap(),
        receipt.revision_id()
    );
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(receipt.revision_id())
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
    let db = rusqlite::Connection::open(f.temp.path().join("data/diskgraph.sqlite")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM collector_runs WHERE collector_id='git-local'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        !f.engine.cancellations().unwrap().contains_key(&job.job_id),
        "reconciled terminal Git job must release its old cancellation handle"
    );
}
