//! Linux 原生执行阶段的真实取消/失权/owner/时限；来源：原任务局部同步点，不构造假 holder。
use crate::process_execution_fixture::{ProcessExecutionFixture, now};
use crate::{EngineConfig, EngineError};
use diskgraph_core::{
    BusinessError, Permission, ProcessEvidenceFailureCode as Code, ProcessEvidenceFailurePhase,
    ProcessEvidenceLimits, Watermark,
};
use diskgraph_store::{JobState, StoreError};
use std::cell::RefCell;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

type Hook = (String, Box<dyn FnOnce()>);
thread_local! {
    static CAPTURE: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static PUBLICATION: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static AFTER_COMMIT: RefCell<Option<Hook>> = const { RefCell::new(None) };
}
fn take(slot: &RefCell<Option<Hook>>, id: &str) {
    let value = if slot.borrow().as_ref().is_some_and(|(job, _)| job == id) {
        slot.borrow_mut().take()
    } else {
        None
    };
    if let Some((_, call)) = value {
        call();
    }
}
pub(super) fn before_capture(id: &str) {
    CAPTURE.with(|slot| take(slot, id));
}
pub(super) fn before_publication(id: &str) {
    PUBLICATION.with(|slot| take(slot, id));
}
pub(super) fn at_publication(id: &str, hook: impl FnOnce() + 'static) {
    PUBLICATION.with(|slot| *slot.borrow_mut() = Some((id.to_owned(), Box::new(hook))));
}
pub(super) fn after_commit(id: &str) -> Result<(), EngineError> {
    let hit = AFTER_COMMIT.with(|slot| {
        let hit = slot.borrow().as_ref().is_some_and(|(job, _)| job == id);
        take(slot, id);
        hit
    });
    if hit {
        Err(BusinessError::Unavailable.into())
    } else {
        Ok(())
    }
}
pub(super) fn at_commit(id: &str, hook: impl FnOnce() + 'static) {
    AFTER_COMMIT.with(|slot| *slot.borrow_mut() = Some((id.to_owned(), Box::new(hook))));
}
pub(super) fn assert_reached() {
    PUBLICATION.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "required actual capture/publication boundary not reached"
        )
    });
}
fn assert_failure(f: &ProcessExecutionFixture, id: &str, state: JobState, code: Code) {
    assert_eq!(f.engine.job_status(id).unwrap().state, state);
    let failure = f
        .engine
        .control_store()
        .unwrap()
        .process_job_failure(id)
        .unwrap()
        .unwrap();
    assert_eq!(failure.phase(), ProcessEvidenceFailurePhase::Execution);
    assert_eq!(failure.code(), code);
    f.assert_no_publication();
    assert!(!f.engine.cancellations().unwrap().contains_key(id));
}
#[test]
fn process_publication_cancel_is_real_cancelled_and_has_no_orphan_run() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let engine = f.engine.clone();
    let actor = f.actor.clone();
    let id = job.job_id.clone();
    at_publication(&job.job_id, move || {
        engine
            .cancel_job(&id, &actor, &engine.policy_authorizer().unwrap())
            .unwrap()
    });
    let result = f.engine.run_job_strict(&job.job_id, "cancel-owner");
    assert_reached();
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Conflict(_)))),
        "{result:?}"
    );
    assert_failure(&f, &job.job_id, JobState::Cancelled, Code::Cancelled);
}
#[test]
fn process_metadata_revocation_after_capture_prevents_publication() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let engine = f.engine.clone();
    let actor = f.actor.clone();
    let scope = f.scope.clone();
    at_publication(&job.job_id, move || {
        engine
            .control_store()
            .unwrap()
            .revoke_grant(&actor, &Permission::MetadataRead, &scope)
            .unwrap()
    });
    let result = f.engine.run_job_strict(&job.job_id, "revoked-owner");
    assert_reached();
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Conflict(_)))),
        "{result:?}"
    );
    assert_failure(&f, &job.job_id, JobState::Failed, Code::Conflict);
}
#[test]
fn process_keeper_observes_index_grant_loss_in_actual_capture_window() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let engine = f.engine.clone();
    let actor = f.actor.clone();
    let scope = f.scope.clone();
    let id = job.job_id.clone();
    CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some((
            id.clone(),
            Box::new(move || {
                let flag = engine.cancellations().unwrap().get(&id).unwrap().clone();
                assert_eq!(engine.job_status(&id).unwrap().state, JobState::Running);
                engine
                    .control_store()
                    .unwrap()
                    .revoke_grant(&actor, &Permission::IndexWrite, &scope)
                    .unwrap();
                let limit = Instant::now() + Duration::from_secs(2);
                while !flag.load(Ordering::SeqCst) {
                    assert!(
                        Instant::now() < limit,
                        "keeper did not observe real grant loss"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }),
        ))
    });
    let result = f.engine.run_job_strict(&job.job_id, "keeper-owner");
    CAPTURE.with(|slot| assert!(slot.borrow().is_none(), "capture boundary not reached"));
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Conflict(_)))),
        "{result:?}"
    );
    assert_failure(&f, &job.job_id, JobState::Failed, Code::Conflict);
}
#[test]
fn process_old_owner_cannot_publish_or_finish_new_owner() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let engine = f.engine.clone();
    let path = f.data.path().join("diskgraph-control.sqlite");
    let id = job.job_id.clone();
    at_publication(&job.job_id, move || {
        assert_eq!(
            rusqlite::Connection::open(path)
                .unwrap()
                .execute(
                    "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
                    [&id]
                )
                .unwrap(),
            1
        );
        engine
            .control_store()
            .unwrap()
            .claim_job_once_strict(&id, "new-owner")
            .unwrap();
    });
    let result = f.engine.run_job_strict(&job.job_id, "old-owner");
    assert_reached();
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::StaleOwner))),
        "{result:?}"
    );
    let current = f.engine.job_status(&job.job_id).unwrap();
    assert_eq!(current.owner, "new-owner");
    assert_eq!(current.state, JobState::Running);
    f.assert_no_publication();
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
}
#[test]
fn process_original_clock_includes_the_capture_preparation_window() {
    let f = ProcessExecutionFixture::new();
    let limits = ProcessEvidenceLimits::new(500, 4 << 20, 32768, 65536, 8 << 20, 2, 64).unwrap();
    let input = f.input(&f.base, limits);
    let job = f
        .engine
        .control_store()
        .unwrap()
        .create_process_evidence_job(&input, &f.authority(now() + 60), 8)
        .unwrap()
        .unwrap();
    CAPTURE.with(|slot| {
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(|| std::thread::sleep(Duration::from_millis(600))),
        ))
    });
    let result = f.engine.run_job_strict(&job.job_id, "clock-owner");
    CAPTURE.with(|slot| assert!(slot.borrow().is_none(), "capture boundary not reached"));
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Timeout))),
        "{result:?}"
    );
    assert_failure(&f, &job.job_id, JobState::Failed, Code::Timeout);
}
#[test]
fn process_tiny_native_budget_refuses_instead_of_publishing_partial_data() {
    let f = ProcessExecutionFixture::new();
    let limits = ProcessEvidenceLimits::new(15000, 1, 32768, 65536, 8 << 20, 2, 64).unwrap();
    let input = f.input(&f.base, limits);
    let job = f
        .engine
        .control_store()
        .unwrap()
        .create_process_evidence_job(&input, &f.authority(now() + 60), 8)
        .unwrap()
        .unwrap();
    let result = f.engine.run_job_strict(&job.job_id, "budget-owner");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "{result:?}"
    );
    assert_failure(&f, &job.job_id, JobState::Failed, Code::BudgetExceeded);
}
#[test]
fn process_capacity_change_after_real_capture_refuses_publication() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let used = f
        .engine
        .capacity_report()
        .iter()
        .map(|r| r.used_bytes)
        .sum::<u64>();
    let engine = crate::Engine::open(EngineConfig {
        data_dir: f.data.path().to_owned(),
        capacity_watermark: Watermark {
            warn_above_bytes: used + 1_000_000,
            refuse_above_bytes: used + 2_000_000,
        },
        ..EngineConfig::default()
    })
    .unwrap();
    let path = f.data.path().join("capacity-fixture");
    at_publication(&job.job_id, move || {
        std::fs::File::create(path)
            .unwrap()
            .set_len(32 << 20)
            .unwrap();
    });
    let result = engine.run_job_strict(&job.job_id, "capacity-owner");
    assert_reached();
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::ResourceExhausted))
        ),
        "{result:?}"
    );
    assert_failure(&f, &job.job_id, JobState::Failed, Code::BudgetExceeded);
    assert!(
        !engine.cancellations().unwrap().contains_key(&job.job_id),
        "actual executing Engine must release its running cancellation handle"
    );
}

// 独立准备预算回归，标准子模块，不改既有十五条阶段断言。
mod preparation_tests;
