//! 完整任务的执行、授权末检和提交事实恢复；来源：真实 Git / SQLite 与请求局部同步点。
use crate::EngineError;
use crate::git_evidence_fixture::{GitEvidenceFixture, now, wait_until};
use diskgraph_core::BusinessError;
use diskgraph_store::JobState;
use std::cell::RefCell;

type Hook = (String, Box<dyn FnOnce()>);
thread_local! {
    static CAPTURE: RefCell<Option<Hook>> = RefCell::new(None);
    static PUBLICATION: RefCell<Option<Hook>> = RefCell::new(None);
    static CRASH_AFTER_COMMIT: RefCell<Option<String>> = const { RefCell::new(None) };
}
fn take(slot: &RefCell<Option<Hook>>, job: &str) {
    let callback = if slot.borrow().as_ref().is_some_and(|(id, _)| id == job) {
        slot.borrow_mut().take()
    } else {
        None
    };
    if let Some((_, callback)) = callback {
        callback();
    }
}
pub(super) fn before_capture(job: &str) {
    CAPTURE.with(|slot| take(slot, job));
}
pub(super) fn before_publication(job: &str) {
    PUBLICATION.with(|slot| take(slot, job));
}
pub(super) fn at_publication(job: &str, callback: impl FnOnce() + 'static) {
    PUBLICATION.with(|slot| *slot.borrow_mut() = Some((job.to_owned(), Box::new(callback))));
}
pub(super) fn crash_after_commit(job: &str) {
    CRASH_AFTER_COMMIT.with(|slot| *slot.borrow_mut() = Some(job.to_owned()));
}
pub(super) fn assert_publication_reached() {
    PUBLICATION.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "did not reach the required post-capture publication boundary"
        )
    });
}
fn assert_capture_reached() {
    CAPTURE.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "did not reach the required active capture boundary"
        )
    });
}
pub(super) fn after_publication(job: &str) -> Result<(), EngineError> {
    let crash = CRASH_AFTER_COMMIT.with(|slot| {
        if slot.borrow().as_deref() == Some(job) {
            slot.borrow_mut().take();
            true
        } else {
            false
        }
    });
    if crash {
        Err(BusinessError::Unavailable.into())
    } else {
        Ok(())
    }
}

#[test]
fn git_content_revocation_after_capture_prevents_any_publication() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let engine = f.engine.clone();
    let scope = f.scope.clone();
    let actor = f.actor.clone();
    PUBLICATION.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || engine.set_content_read(&scope, &actor, false).unwrap()),
        ))
    });
    let error = f.engine.run_job_strict(&job.job_id, "revoked").unwrap_err();
    assert_publication_reached();
    assert!(
        matches!(
            error,
            EngineError::Store(diskgraph_store::StoreError::Conflict(_))
        ),
        "{error:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    f.assert_no_git_publication();
}

#[test]
fn git_actual_cancellation_after_capture_is_cancelled_without_publication() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let engine = f.engine.clone();
    let actor = f.actor.clone();
    let id = job.job_id.clone();
    PUBLICATION.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                engine
                    .cancel_job(&id, &actor, &engine.policy_authorizer().unwrap())
                    .unwrap()
            }),
        ))
    });
    let error = f
        .engine
        .run_job_strict(&job.job_id, "cancelled")
        .unwrap_err();
    assert_publication_reached();
    assert!(
        matches!(
            error,
            EngineError::Store(diskgraph_store::StoreError::Conflict(_))
        ),
        "{error:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Cancelled
    );
    let failure = f
        .engine
        .git_job_failure(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(failure).unwrap(),
        serde_json::json!({"phase":"execution","code":"cancelled"})
    );
    f.assert_no_git_publication();
}

#[test]
fn git_original_request_expiry_after_capture_fails_without_publication() {
    let f = GitEvidenceFixture::new();
    // 捕获允许消耗完整的服务端运行预算；测试 token 必须留出到达发布边界的窗口。
    // 原 exp 在入队前固定，后续既不修改 authority，也不续租 token。
    let expiry = now()
        + diskgraph_core::GitEvidenceLimits::default()
            .max_duration_ms()
            .div_ceil(1000)
        + 5;
    let job = f.enqueue(&f.base, expiry);
    let original = f
        .engine
        .control_store()
        .unwrap()
        .job_request_authority(&job.job_id)
        .unwrap()
        .unwrap();
    let engine = f.engine.clone();
    let id = job.job_id.clone();
    let captured_authority = original.clone();
    PUBLICATION.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                assert_eq!(engine.job_status(&id).unwrap().state, JobState::Running);
                assert_eq!(
                    engine
                        .control_store()
                        .unwrap()
                        .job_request_authority(&id)
                        .unwrap(),
                    Some(captured_authority.clone())
                );
                assert!(
                    now() < expiry,
                    "request expired before the publication fixture boundary"
                );
                // 仅在真实捕获成功后推进真实时间；有界等待不依赖 Windows 在三秒内完成 Git。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
                while now() < expiry {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "request expiry wait exceeded its fixture budget"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                assert_eq!(
                    captured_authority.validate_at(now()),
                    Err(BusinessError::PermissionDenied)
                );
            }),
        ))
    });
    let started = std::time::Instant::now();
    let error = f.engine.run_job_strict(&job.job_id, "expired").unwrap_err();
    PUBLICATION.with(|slot| assert!(slot.borrow().is_none(),
        "did not reach the required post-capture publication boundary: original_expiry={expiry}, now={}, elapsed={:?}, error={error:?}", now(), started.elapsed()));
    assert_publication_reached();
    assert!(
        matches!(
            &error,
            EngineError::Store(diskgraph_store::StoreError::Conflict(_))
        ),
        "{error:?}"
    );
    assert!(
        matches!(&error, EngineError::Store(diskgraph_store::StoreError::Conflict(message)) if message == "job request authority denied"),
        "expiry must be rejected by the real persistent authority gate: {error:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .job_request_authority(&job.job_id)
            .unwrap(),
        Some(original)
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
    assert_eq!(
        diagnostic.phase(),
        diskgraph_core::GitEvidenceFailurePhase::Execution
    );
    assert_eq!(
        diagnostic.code(),
        diskgraph_core::GitEvidenceFailureCode::Conflict
    );
    f.assert_no_git_publication();
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
}

#[test]
fn git_job_cannot_publish_for_a_replacement_directory_after_enqueue() {
    let f = GitEvidenceFixture::new();
    let replacement = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    std::fs::rename(f.temp.path().join("repo"), f.temp.path().join("original")).unwrap();
    std::fs::rename(
        replacement.temp.path().join("repo"),
        f.temp.path().join("repo"),
    )
    .unwrap();
    assert!(f.engine.run_job_strict(&job.job_id, "replacement").is_err());
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    f.assert_no_git_publication();
}

#[test]
fn git_owner_loss_after_capture_cannot_publish_or_finish_the_new_owner() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let engine = f.engine.clone();
    let path = f.temp.path().join("data/diskgraph-control.sqlite");
    let id = job.job_id.clone();
    PUBLICATION.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                rusqlite::Connection::open(path)
                    .unwrap()
                    .execute(
                        "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
                        [&id],
                    )
                    .unwrap();
                engine
                    .control_store()
                    .unwrap()
                    .claim_job_once_strict(&id, "new-owner")
                    .unwrap();
            }),
        ))
    });
    let error = f
        .engine
        .run_job_strict(&job.job_id, "old-owner")
        .unwrap_err();
    assert_publication_reached();
    assert!(
        matches!(
            error,
            EngineError::Store(diskgraph_store::StoreError::StaleOwner)
        ),
        "{error:?}"
    );
    let current = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(current.state, JobState::Running);
    assert_eq!(current.owner, "new-owner");
    f.assert_no_git_publication();
}

#[test]
fn git_input_budget_failure_is_typed_and_does_not_publish_partial_counts() {
    use diskgraph_core::{GitEvidenceJobInput, GitEvidenceLimits, JobRequestAuthority};
    let f = GitEvidenceFixture::new();
    let limits = GitEvidenceLimits::new(15_000, 1 << 20, 1, 32768, 128 << 20, 64 << 20).unwrap();
    let input = GitEvidenceJobInput::new(
        f.engine.server_id().unwrap(),
        f.scope.clone(),
        f.base.clone(),
        f.node,
        limits,
    )
    .unwrap();
    let authority =
        JobRequestAuthority::trusted_local(f.actor.clone(), "trusted-test-adapter").unwrap();
    let job = f
        .engine
        .control_store()
        .unwrap()
        .create_git_evidence_job(&input, &authority, 10)
        .unwrap()
        .unwrap();
    let error = f.engine.run_job(&job.job_id, "input-budget").unwrap_err();
    assert!(
        matches!(error, EngineError::Business(BusinessError::BudgetExceeded)),
        "{error:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    let failure = f
        .engine
        .git_job_failure(
            &job.job_id,
            &f.actor,
            &f.engine.policy_authorizer().unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(failure).unwrap(),
        serde_json::json!({"phase":"execution","code":"budget_exceeded"})
    );
    f.assert_no_git_publication();
}

#[test]
fn git_claim_preparation_keeps_the_original_absolute_execution_deadline() {
    use diskgraph_core::{GitEvidenceJobInput, GitEvidenceLimits, JobRequestAuthority};
    let f = GitEvidenceFixture::new();
    let limits = GitEvidenceLimits::new(50, 1 << 20, 64 << 20, 32768, 128 << 20, 64 << 20).unwrap();
    let input = GitEvidenceJobInput::new(
        f.engine.server_id().unwrap(),
        f.scope.clone(),
        f.base.clone(),
        f.node,
        limits,
    )
    .unwrap();
    let authority =
        JobRequestAuthority::trusted_local(f.actor.clone(), "trusted-test-adapter").unwrap();
    let job = f
        .engine
        .control_store()
        .unwrap()
        .create_git_evidence_job(&input, &authority, 10)
        .unwrap()
        .unwrap();
    CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(|| std::thread::sleep(std::time::Duration::from_millis(100))),
        ))
    });
    let error = f
        .engine
        .run_job(&job.job_id, "absolute-deadline")
        .unwrap_err();
    assert_capture_reached();
    assert!(
        matches!(error, EngineError::Business(BusinessError::Timeout)),
        "{error:?}"
    );
    f.assert_no_git_publication();
}

#[test]
fn git_keeper_observes_content_revocation_during_the_active_execution_window() {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let engine = f.engine.clone();
    let scope = f.scope.clone();
    let actor = f.actor.clone();
    let id = job.job_id.clone();
    CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                assert_eq!(engine.job_status(&id).unwrap().state, JobState::Running);
                let flag = engine.cancellations().unwrap().get(&id).unwrap().clone();
                engine.set_content_read(&scope, &actor, false).unwrap();
                let limit = Instant::now() + Duration::from_secs(2);
                while !flag.load(Ordering::SeqCst) {
                    assert!(
                        Instant::now() < limit,
                        "keeper did not observe ContentRead loss"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }),
        ))
    });
    let error = f
        .engine
        .run_job_strict(&job.job_id, "live-revocation")
        .unwrap_err();
    assert_capture_reached();
    assert!(
        matches!(
            error,
            EngineError::Store(diskgraph_store::StoreError::Conflict(_))
        ),
        "{error:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    f.assert_no_git_publication();
}

#[test]
fn expired_queued_git_claim_releases_its_terminal_cancellation_handle() {
    let f = GitEvidenceFixture::new();
    let expiry = now() + 2;
    let job = f.enqueue(&f.base, expiry);
    assert_eq!(job.state, JobState::Queued);
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    wait_until(expiry);
    let result = f.engine.run_job_strict(&job.job_id, "expired-queued-owner");
    assert!(matches!(
        result,
        Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
    ));
    let terminal = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(terminal.state, JobState::Failed);
    assert_eq!(
        terminal.fencing_token, 0,
        "expired queued request must not enter execution"
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
    assert_eq!(
        diagnostic.phase(),
        diskgraph_core::GitEvidenceFailurePhase::Admission
    );
    assert_eq!(
        diagnostic.code(),
        diskgraph_core::GitEvidenceFailureCode::Conflict
    );
    f.assert_no_git_publication();
    assert!(
        !f.engine.cancellations().unwrap().contains_key(&job.job_id),
        "strict claim terminal rejection must release the original queued cancellation handle"
    );
}
