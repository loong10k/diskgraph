//! 成功 Git 发布与图已提交事实恢复；来源：真实任务、Git 观察和不可变 publication receipt。
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
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
    // 原 token 固定覆盖服务端 15 秒运行窗；不在捕获后续期。
    let expiry = now() + 20;
    let job = f.enqueue(&f.base, expiry);
    crate::git_evidence_execution_tests::crash_after_commit(&job.job_id);
    let original_authority = f
        .engine
        .control_store()
        .unwrap()
        .job_request_authority(&job.job_id)
        .unwrap()
        .unwrap();
    let outcome = f.engine.run_job_strict(&job.job_id, "crashed-owner");
    let state = f.engine.job_status(&job.job_id).unwrap();
    let failure = f
        .engine
        .control_store()
        .unwrap()
        .git_job_failure(&job.job_id)
        .unwrap();
    let observed_receipt = f
        .engine
        .revision_reader()
        .unwrap()
        .job_publication_receipt(&job.job_id)
        .unwrap();
    eprintln!(
        "recovery prerequisite: outcome={outcome:?}, state={state:?}, failure={failure:?}, receipt={observed_receipt:?}, expiry={expiry}, now={}",
        now()
    );
    assert!(
        matches!(
            outcome,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::Unavailable
            ))
        ),
        "only the actual post-commit simulated crash qualifies: {outcome:?}"
    );
    crate::git_evidence_execution_tests::assert_crash_after_commit_reached(&job.job_id);
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
    assert_eq!(state.owner, "crashed-owner");
    assert_eq!(state.fencing_token, receipt.publishing_fence());
    assert!(state.lease_expires_unix_ms > now() * 1000);
    assert!(receipt.committed_at_unix_ms() < expiry * 1000);
    assert!(failure.is_none());
    assert_eq!(receipt.input().base_revision_id(), f.base);
    assert_eq!(original_authority.expires_at_unix_seconds(), Some(expiry));
    // 明确模拟进程崩溃后的租约流逝，只推进租约，不伪造输入/authority/receipt。
    rusqlite::Connection::open(f.temp.path().join("data/diskgraph-control.sqlite"))
        .unwrap()
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    // 只等待此请求原 exp 自然到达；独立等待上限不改变其他夹具的五秒 helper。
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(expiry.saturating_sub(now()) + 5);
    while now() < expiry {
        assert!(
            std::time::Instant::now() < deadline,
            "original expiry wait exceeded fixture bound"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        original_authority.validate_at(now()),
        Err(diskgraph_core::BusinessError::PermissionDenied)
    );
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .job_request_authority(&job.job_id)
            .unwrap(),
        Some(original_authority)
    );
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
