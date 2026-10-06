//! 真实Cargo helper→sealed→Atomic→driver→Engine持久发布；来源：PF-06。
//! 必须在Linux实际执行；macOS构建不构成这些场景的验收，不查PATH或跳过缺artifact。
#![cfg(target_os = "linux")]

#[path = "scan_worker_engine_flow/fixture.rs"]
mod fixture;
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_store::JobState;
use fixture::{Fixture, limits};
use std::path::Path;

#[test]
fn held_actual_helper_survives_path_replacement_and_publishes_original_job_revision() {
    let fixture = Fixture::new(limits(), false, true);
    let job = fixture.enqueue();
    assert_eq!(job.state, JobState::Queued);
    let result = fixture.engine.run_job(&job.job_id, "actual-helper-owner");
    let drained = fixture.recovery.drain();
    let occupied = fixture.recovery.occupied_slots();
    let completed = result.unwrap();
    assert_eq!(completed.state, JobState::Completed);
    assert!(completed.fencing_token > 0);
    let revision = fixture
        .engine
        .revision_for_job(&job.job_id, &fixture.actor, &fixture.policy)
        .unwrap();
    assert_eq!(
        fixture.engine.latest_revision(&fixture.scope).unwrap(),
        Some(revision.clone())
    );
    let first = fixture
        .engine
        .revision_node_at(&revision, Path::new("one"))
        .unwrap()
        .unwrap();
    let second = fixture
        .engine
        .revision_node_at(&revision, Path::new("nested/two"))
        .unwrap()
        .unwrap();
    assert!(!first.read_error && !second.read_error);
    assert!(fixture.root.join("one").is_file());
    assert!(
        fixture
            .directory
            .path()
            .join("original-image-moved")
            .is_file()
    );
    assert_eq!(occupied.unwrap(), 0);
    assert!(
        drained.unwrap(),
        "normal completion already consumed actual original wait"
    );
}

#[test]
fn wrong_independent_image_hash_is_conflict_without_birth_or_publication() {
    let fixture = Fixture::new(limits(), true, false);
    let job = fixture.enqueue();
    let result = fixture.engine.run_job(&job.job_id, "wrong-image-owner");
    let drained = fixture.recovery.drain();
    assert!(
        matches!(
            result.as_ref().err().map(EngineError::primary),
            Some(EngineError::Business(BusinessError::Conflict))
        ),
        "{result:?}"
    );
    assert_eq!(
        fixture.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_eq!(
        fixture.engine.latest_revision(&fixture.scope).unwrap(),
        None
    );
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
    assert!(drained.unwrap());
}

#[test]
fn response_ledger_exhaustion_never_publishes_a_partial_revision_or_leaks_owner() {
    let mut response = limits();
    response.max_stream_bytes = 32;
    let fixture = Fixture::new(response, false, false);
    let job = fixture.enqueue();
    let result = fixture
        .engine
        .run_job(&job.job_id, "original-output-budget-owner");
    let drained = fixture.recovery.drain();
    assert!(
        result.is_err(),
        "original tiny response total must reject the actual helper output"
    );
    assert_eq!(
        fixture.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_eq!(
        fixture.engine.latest_revision(&fixture.scope).unwrap(),
        None
    );
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
    assert!(
        drained.unwrap(),
        "error cleanup consumed original wait, not a normal-result permit"
    );
}
