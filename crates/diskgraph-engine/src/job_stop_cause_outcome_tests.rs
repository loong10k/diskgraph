//! 独立错误和已提交事实不能被晚到的 keeper 原因覆盖；来源：实际 Git 采样与图库回执。

use crate::EngineError;
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use crate::git_evidence_target::GitEvidenceTarget;
use crate::job_stop_cause_fixture::{
    assert_failed_without_caller_cancel, lease_live, qualify_claim, withdraw,
};
use crate::job_stop_cause_hooks as hooks;
use crate::live_evidence::{EvidenceProbeSession, ProbeLimits};
use diskgraph_core::{BusinessError, Permission};
use diskgraph_store::{JobState, StoreError};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn real_identity_conflict_is_not_replaced_by_a_later_keeper_authorization_error() {
    let f = Arc::new(GitEvidenceFixture::new());
    let other = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let input = f
        .engine
        .control_store()
        .unwrap()
        .git_evidence_job_input(&job.job_id)
        .unwrap();
    let target = GitEvidenceTarget::load(
        &f.engine,
        &input,
        Instant::now() + Duration::from_secs(15),
        None,
    )
    .unwrap();
    let root = f
        .engine
        .control_store()
        .unwrap()
        .scope(&f.scope)
        .unwrap()
        .root
        .to_native_path()
        .unwrap();
    let other_root = other
        .engine
        .control_store()
        .unwrap()
        .scope(&other.scope)
        .unwrap()
        .root
        .to_native_path()
        .unwrap();
    let retained = f.temp.path().join("retained-original-repo");
    let identity_seen = Arc::new(AtomicBool::new(false));
    let identity = identity_seen.clone();
    crate::git_evidence_execution_tests::at_capture(&job.job_id, move || {
        let limits = ProbeLimits::default();
        let mut unchanged = EvidenceProbeSession::new(&limits).unwrap();
        assert_eq!(
            unchanged
                .sample_git_indexed(Path::new("git"), &root, &target.locator, &target.identity)
                .unwrap()
                .dirty_count,
            0
        );
        std::fs::rename(&root, retained).unwrap();
        std::fs::rename(other_root, &root).unwrap();
        let mut changed = EvidenceProbeSession::new(&limits).unwrap();
        let error = changed
            .sample_git_indexed(Path::new("git"), &root, &target.locator, &target.identity)
            .unwrap_err();
        assert_eq!(format!("{error:?}"), "IdentityChanged");
        assert_eq!(error.business(), BusinessError::Conflict);
        assert!(!limits.cancel.load(Ordering::SeqCst));
        identity.store(true, Ordering::SeqCst);
    });
    let (failed, keeper_failed) = std::sync::mpsc::channel();
    hooks::at_keeper_error(&job.job_id, move |claimed, error| {
        failed.send((matches!(error, EngineError::Store(StoreError::Conflict(message)) if message == "live job authorization withdrawn"), lease_live(claimed))).unwrap();
    });
    let fixture = f.clone();
    let source = identity_seen.clone();
    let later_seen = Arc::new(AtomicBool::new(false));
    let later = later_seen.clone();
    hooks::at_outcome(&job.job_id, move |result| {
        // 独立 native 结果已由 actual executor 返回，之后才撤权；不注入 EngineError。
        assert!(source.load(Ordering::SeqCst));
        assert!(
            matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
            "actual identity result absent: {result:?}"
        );
        withdraw(&fixture, Permission::ContentRead);
        let (denied, live) = keeper_failed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(live);
        later.store(denied, Ordering::SeqCst);
    });
    let result = f
        .engine
        .run_job_strict(&job.job_id, "identity-before-keeper");
    hooks::clear();
    assert!(identity_seen.load(Ordering::SeqCst));
    assert!(later_seen.load(Ordering::SeqCst));
    assert_failed_without_caller_cancel(&f, &job.job_id, "identity-before-keeper");
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
        "later cooperative-stop cause replaced an independent native result: {result:?}"
    );
}

#[test]
fn a_real_committed_receipt_remains_completed_after_late_keeper_denial() {
    let f = Arc::new(GitEvidenceFixture::new());
    let job = f.enqueue(&f.base, now() + 300);
    let (failed, keeper_failed) = std::sync::mpsc::channel();
    hooks::at_keeper_error(&job.job_id, move |claimed, error| {
        failed.send((matches!(error, EngineError::Store(StoreError::Conflict(message)) if message == "live job authorization withdrawn"), lease_live(claimed))).unwrap();
    });
    let fixture = f.clone();
    let id = job.job_id.clone();
    let commit_seen = Arc::new(AtomicBool::new(false));
    let committed = commit_seen.clone();
    hooks::at_outcome(&job.job_id, move |result| {
        assert!(
            result.is_ok(),
            "actual collector commit not reached: {result:?}"
        );
        let receipt = fixture
            .engine
            .graph()
            .unwrap()
            .job_publication_receipt(&id)
            .unwrap()
            .unwrap();
        assert_eq!(
            fixture
                .engine
                .latest_revision(&fixture.scope)
                .unwrap()
                .as_deref(),
            Some(receipt.revision_id())
        );
        assert_eq!(receipt.job_id(), id);
        // 这里观察提交事实，不声称撤权以后允许新采集或新发布。
        committed.store(true, Ordering::SeqCst);
        withdraw(&fixture, Permission::ContentRead);
        let (denied, live) = keeper_failed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            denied && live,
            "late keeper must report a real live-lease authorization loss"
        );
    });
    let result = f
        .engine
        .run_job_strict(&job.job_id, "committed-before-keeper");
    hooks::clear();
    assert!(commit_seen.load(Ordering::SeqCst));
    let completed = result.expect("already committed fact must not become a failed job");
    assert_eq!(completed.state, JobState::Completed);
    let receipt = f
        .engine
        .graph()
        .unwrap()
        .job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.publishing_fence(), completed.fencing_token);
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(receipt.revision_id())
    );
    assert_eq!(f.engine.job_status(&job.job_id).unwrap(), completed);
    assert!(
        !f.engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, completed.fencing_token)
            .unwrap()
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    let connection =
        rusqlite::Connection::open(f.engine.data_dir().join("diskgraph.sqlite")).unwrap();
    let runs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM collector_runs WHERE collector_id='git-local'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, 1, "late denial must not repeat collector execution");
}

#[test]
fn keeper_join_propagates_its_real_panic_without_rewriting_committed_facts() {
    let f = Arc::new(GitEvidenceFixture::new());
    let job = f.enqueue(&f.base, now() + 300);
    let fixture = f.clone();
    let (qualified, keeper_qualified) = std::sync::mpsc::channel();
    hooks::before_keeper(&job.job_id, move |claimed| {
        qualify_claim(&fixture, claimed, "keeper-panic");
        qualified.send(claimed.fencing_token).unwrap();
        // 普通 Rust 观察点在 DB 锁外 panic；不是 SQLite 回调或伪造 SQL 错误。
        std::panic::panic_any("qualified keeper panic");
    });
    let phase_seen = Arc::new(AtomicBool::new(false));
    let phase = phase_seen.clone();
    crate::git_evidence_execution_tests::at_publication(&job.job_id, move || {
        let fence = keeper_qualified
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(fence > 0);
        phase.store(true, Ordering::SeqCst);
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.engine.run_job_strict(&job.job_id, "keeper-panic")
    }));
    hooks::clear();
    crate::git_evidence_execution_tests::assert_publication_reached(&result);
    assert!(phase_seen.load(Ordering::SeqCst));
    let receipt = f
        .engine
        .graph()
        .unwrap()
        .job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(receipt.revision_id())
    );
    let record = f.engine.job_status(&job.job_id).unwrap();
    assert!(
        matches!(record.state, JobState::Running | JobState::Completed),
        "panic cannot erase already committed facts: {record:?}"
    );
    assert!(
        !f.engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, record.fencing_token)
            .unwrap()
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    assert!(
        !f.engine
            .scan_progress
            .lock()
            .unwrap()
            .keys()
            .any(|(id, _)| id == &job.job_id)
    );
    let panic = result.expect_err("keeper.join must not silently discard a real thread panic");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"qualified keeper panic")
    );
}
