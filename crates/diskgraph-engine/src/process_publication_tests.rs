//! 真实 Linux 发布、旧正向来源与已提交事实恢复；来源：原生 tmpfs/SQLite 与原不可变请求期限。
use crate::EngineError;
use crate::process_execution_fixture::{ProcessExecutionFixture, now};
use crate::process_execution_tests::{assert_reached, at_commit, at_publication};
use diskgraph_core::{BusinessError, Permission, ProcessEvidenceSummary, Relation};
use diskgraph_store::{JobState, StoreError};
use std::time::{Duration, Instant};

#[test]
fn process_partial_refresh_keeps_prior_positive_holder_assertions() {
    let f = ProcessExecutionFixture::new();
    let held = std::fs::File::open(f.source.path().join("container/scope/target")).unwrap();
    let first = f.enqueue(&f.base, now() + 60);
    f.engine
        .run_job_strict(&first.job_id, "first-owner")
        .unwrap();
    let reader = f.engine.revision_reader().unwrap();
    let first_receipt = reader
        .process_job_publication_receipt(&first.job_id)
        .unwrap()
        .unwrap();
    let old = reader
        .revision_evidence(first_receipt.revision_id())
        .unwrap()
        .all_edges()
        .unwrap();
    let own_edges = old
        .iter()
        .filter(|edge| edge.relation == Relation::UsedByProcess)
        .filter(|edge| {
            let entity = reader
                .revision_evidence(first_receipt.revision_id())
                .unwrap()
                .entity(&edge.target_entity_id)
                .unwrap()
                .unwrap();
            let identity: serde_json::Value = serde_json::from_str(&entity.identity).unwrap();
            identity["startup"]["pid"].as_u64() == Some(u64::from(std::process::id()))
        })
        .map(|edge| edge.edge_id.clone())
        .collect::<Vec<_>>();
    assert!(!own_edges.is_empty(), "first real holder must be observed");
    drop(held);
    drop(reader);
    let second = f.enqueue(first_receipt.revision_id(), now() + 60);
    f.engine
        .run_job_strict(&second.job_id, "second-owner")
        .unwrap();
    let reader = f.engine.revision_reader().unwrap();
    let receipt = reader
        .process_job_publication_receipt(&second.job_id)
        .unwrap()
        .unwrap();
    let selected = reader.revision_evidence(receipt.revision_id()).unwrap();
    let edges = selected.all_edges().unwrap();
    assert!(
        own_edges
            .iter()
            .all(|id| edges.iter().any(|edge| &edge.edge_id == id)),
        "partial refresh must preserve old positive sources"
    );
    let evidence = selected
        .evidence_record_with_budget(
            &format!("{}-summary", receipt.run_id()),
            &mut diskgraph_core::QueryReadBudget::new(
                diskgraph_core::QueryBudget::default(),
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    let summary: ProcessEvidenceSummary = serde_json::from_str(&evidence.basis).unwrap();
    assert!(
        summary
            .processes()
            .iter()
            .all(|p| p.pid() != std::process::id()),
        "observer-only FD must not become a new holder"
    );
    assert_eq!(
        summary.coverage(),
        diskgraph_core::ProcessObservationCoverage::Partial
    );
    assert_eq!(receipt.snapshot_id(), first_receipt.snapshot_id());
}

#[test]
fn process_live_latest_cas_after_capture_rolls_back_the_entire_batch() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let engine = f.engine.clone();
    let actor = f.actor.clone();
    let scope = f.scope.clone();
    at_publication(&job.job_id, move || {
        let auth = engine.policy_authorizer().unwrap();
        let scan = engine.sync_scope(&scope, &actor, &auth).unwrap();
        engine.run_job(&scan.job_id, "newer-scan").unwrap();
    });
    let result = f.engine.run_job_strict(&job.job_id, "stale-base");
    assert_reached();
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Conflict(_)))),
        "{result:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_ne!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(f.base.as_str())
    );
    let graph = f.engine.revision_reader().unwrap();
    assert!(
        graph
            .process_job_publication_receipt(&job.job_id)
            .unwrap()
            .is_none()
    );
    let db = rusqlite::Connection::open(f.data.path().join("diskgraph.sqlite")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM collector_runs WHERE collector_id='process-native'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "CAS failure must not leave orphan process run");
}

#[test]
fn process_original_token_expiry_after_capture_prevents_publication() {
    let f = ProcessExecutionFixture::new();
    let expiry = now()
        + diskgraph_core::ProcessEvidenceLimits::default()
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
    let authority = original.clone();
    at_publication(&job.job_id, move || {
        assert!(
            now() < expiry,
            "did not reach publication with the original token still live"
        );
        assert_eq!(
            engine
                .control_store()
                .unwrap()
                .job_request_authority(&id)
                .unwrap(),
            Some(authority.clone())
        );
        let limit = Instant::now() + Duration::from_secs(25);
        while now() < expiry {
            assert!(Instant::now() < limit);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            authority.validate_at(now()),
            Err(BusinessError::PermissionDenied)
        );
    });
    let result = f.engine.run_job_strict(&job.job_id, "expiry-owner");
    assert_reached();
    assert!(
        matches!(result,Err(EngineError::Store(StoreError::Conflict(ref reason))) if reason=="job request authority denied"),
        "{result:?}"
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
    f.assert_no_publication();
}

#[test]
fn process_committed_receipt_recovers_after_expiry_without_opening_removed_source() {
    let f = ProcessExecutionFixture::new();
    let expiry = now() + 20;
    let job = f.enqueue(&f.base, expiry);
    let engine = f.engine.clone();
    let id = job.job_id.clone();
    let path = f.data.path().join("diskgraph-control.sqlite");
    at_commit(&job.job_id, move || {
        assert!(
            engine
                .revision_reader()
                .unwrap()
                .process_job_publication_receipt(&id)
                .unwrap()
                .is_some(),
            "commit hook requires durable graph fact"
        );
        assert!(now() < expiry, "fixture token expired before actual commit");
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
    });
    let result = f.engine.run_job_strict(&job.job_id, "lost-control-finish");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::Unavailable))
        ),
        "{result:?}"
    );
    let reader = f.engine.revision_reader().unwrap();
    let receipt = reader
        .process_job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    drop(reader);
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Running
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    let limit = Instant::now() + Duration::from_secs(25);
    while now() < expiry {
        assert!(Instant::now() < limit);
        std::thread::sleep(Duration::from_millis(5));
    }
    f.engine
        .control_store()
        .unwrap()
        .revoke_grant(&f.actor, &Permission::IndexWrite, &f.scope)
        .unwrap();
    std::fs::remove_dir_all(f.source.path().join("container")).unwrap();
    let recovered = f
        .engine
        .run_job_strict(&job.job_id, "reconcile-owner")
        .unwrap();
    assert_eq!(recovered.state, JobState::Completed);
    assert!(recovered.fencing_token > receipt.publishing_fence());
    assert_eq!(
        f.engine
            .revision_reader()
            .unwrap()
            .process_job_publication_receipt(&job.job_id)
            .unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(receipt.revision_id())
    );
    let db = rusqlite::Connection::open(f.data.path().join("diskgraph.sqlite")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM collector_runs WHERE collector_id='process-native'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "recovery must not resample or publish another run"
    );
}

#[test]
fn process_target_replaced_after_observation_and_encoding_cannot_publish() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let root = f.source.path().join("container/scope");
    at_publication(&job.job_id, move || {
        std::fs::rename(root.join("target"), root.join("retained-original")).unwrap();
        std::fs::write(root.join("target"), b"different replacement inode").unwrap();
    });
    let result = f
        .engine
        .run_job_strict(&job.job_id, "late-target-replacement");
    assert_reached();
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
        "encoded observation must still verify its target before publication: {result:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .process_job_failure(&job.job_id)
            .unwrap()
            .unwrap()
            .code(),
        diskgraph_core::ProcessEvidenceFailureCode::Conflict
    );
    f.assert_no_publication();
}

#[test]
fn process_registered_root_replaced_after_observation_and_encoding_cannot_publish() {
    let f = ProcessExecutionFixture::new();
    let job = f.enqueue(&f.base, now() + 60);
    let container = f.source.path().join("container");
    at_publication(&job.job_id, move || {
        std::fs::rename(container.join("scope"), container.join("retained-scope")).unwrap();
        std::fs::create_dir(container.join("scope")).unwrap();
        std::fs::write(
            container.join("scope/target"),
            b"replacement registered root",
        )
        .unwrap();
    });
    let result = f
        .engine
        .run_job_strict(&job.job_id, "late-root-replacement");
    assert_reached();
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
        "encoded observation must still verify its registered root before publication: {result:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .process_job_failure(&job.job_id)
            .unwrap()
            .unwrap()
            .code(),
        diskgraph_core::ProcessEvidenceFailureCode::Conflict
    );
    f.assert_no_publication();
}
