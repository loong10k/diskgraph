//! D42 真正 Linux 任务执行验收；来源：公开 scan、实际持久 epoch、typed Control 队列与 Engine runner。
//! 当前临时 Unsupported foundation 必须在 positive case 失败；预检缺能力不能算通过。
#![cfg(target_os = "linux")]
use diskgraph_core::{
    BusinessError, JobRequestAuthority, Permission, PrincipalId, ProcessEvidenceFailureCode,
    ProcessEvidenceJobInput, ProcessEvidenceLimits, ProcessEvidenceSummary,
    ProcessObservationCoverage, ProcessObservationMethod, QueryBudget, QueryReadBudget, Relation,
    ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_store::{ControlStore, JobRecord, JobState, SqliteSnapshotStore};
use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

/// 真 tmpfs 源与独立持久图/控制库；来源：Linux 原生 Engine 扫描，身份绝不由测试合成。
struct ExecutionFixture {
    source: tempfile::TempDir,
    data: tempfile::TempDir,
    engine: Engine,
    actor: PrincipalId,
    scope: ScopeId,
    base: String,
    input: ProcessEvidenceJobInput,
}
impl ExecutionFixture {
    fn new() -> Self {
        let source = tempfile::tempdir_in("/dev/shm").expect("native Linux test requires tmpfs");
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("target"),
            b"content is never collector input",
        )
        .unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: data.path().to_owned(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("linux-process-execution").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let scope = engine
            .register_scope(source.path(), &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        let auth = engine.policy_authorizer().unwrap();
        let scan = engine.index_scope(&scope, &actor, &auth).unwrap();
        assert_eq!(
            engine.run_job(&scan.job_id, "index-worker").unwrap().state,
            JobState::Completed
        );
        let base = engine
            .revision_for_job(&scan.job_id, &actor, &auth)
            .unwrap();
        let node = engine
            .revision_node_at(&base, Path::new("target"))
            .unwrap()
            .unwrap();
        let snapshot = engine.revision_snapshot(&base).unwrap();
        let graph = SqliteSnapshotStore::open(&data.path().join("diskgraph.sqlite")).unwrap();
        let mut reads = QueryReadBudget::new(
            QueryBudget::default(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let captured = graph
            .unix_observation_bounded(&snapshot.id, node.id, &mut reads)
            .unwrap()
            .unwrap();
        assert!(
            captured.gap.is_none(),
            "native prerequisites not verified: {:?}",
            captured.gap
        );
        let input = ProcessEvidenceJobInput::new(
            engine.server_id().unwrap(),
            scope.clone(),
            base.clone(),
            node.id,
            ProcessObservationMethod::LinuxProcfsV1,
            captured.observation.unwrap().epoch().clone(),
            ProcessEvidenceLimits::default(),
        )
        .unwrap();
        Self {
            source,
            data,
            engine,
            actor,
            scope,
            base,
            input,
        }
    }
    fn enqueue(&self) -> JobRecord {
        let authority = JobRequestAuthority::authenticated_remote(
            self.actor.clone(),
            "execution-fixture",
            "http",
            vec![Permission::MetadataRead, Permission::IndexWrite],
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 60,
        )
        .unwrap();
        ControlStore::open(&self.data.path().join("diskgraph-control.sqlite"))
            .unwrap()
            .create_process_evidence_job(&self.input, &authority, 8)
            .unwrap()
            .unwrap()
    }
    fn graph(&self) -> SqliteSnapshotStore {
        SqliteSnapshotStore::open(&self.data.path().join("diskgraph.sqlite")).unwrap()
    }
}

#[test]
fn real_linux_holder_job_publishes_same_snapshot_revision_and_reconnect_receipt_without_content_grant()
 {
    let f = ExecutionFixture::new();
    let held = File::open(f.source.path().join("target")).unwrap();
    let job = f.enqueue();
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(f.base.as_str())
    );
    assert!(
        f.graph()
            .process_job_publication_receipt(&job.job_id)
            .unwrap()
            .is_none()
    );
    let result = f.engine.run_job_strict(&job.job_id, "process-worker");
    assert!(
        result.is_ok(),
        "native execution must replace Unsupported foundation: {result:?}"
    );
    assert_eq!(result.unwrap().state, JobState::Completed);
    let graph = f.graph();
    let receipt = graph
        .process_job_publication_receipt(&job.job_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.base_revision_id(), f.base);
    assert_eq!(
        receipt.snapshot_id(),
        f.engine.revision_snapshot(&f.base).unwrap().id
    );
    assert_eq!(receipt.input(), &f.input);
    assert_eq!(receipt.scope_id(), &f.scope);
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(receipt.revision_id())
    );
    let selected = graph.revision_evidence(receipt.revision_id()).unwrap();
    let edges = selected
        .all_edges()
        .unwrap()
        .into_iter()
        .filter(|e| e.relation == Relation::UsedByProcess)
        .collect::<Vec<_>>();
    assert!(
        !edges.is_empty(),
        "real open file must produce a positive edge"
    );
    let mut ours = false;
    for edge in &edges {
        let process = selected.entity(&edge.target_entity_id).unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&process.identity).unwrap();
        ours |= value["startup"]["pid"].as_u64() == Some(u64::from(std::process::id()));
    }
    assert!(
        ours,
        "same-PID real holder cannot be excluded with observer FD"
    );
    let evidence = selected
        .evidence_for_edges(&edges.iter().map(|e| e.edge_id.clone()).collect::<Vec<_>>())
        .unwrap();
    let summary: ProcessEvidenceSummary = serde_json::from_str(
        &evidence
            .iter()
            .find(|e| e.run_id == receipt.run_id())
            .unwrap()
            .basis,
    )
    .unwrap();
    assert_eq!(summary.coverage(), ProcessObservationCoverage::Partial);
    assert!(
        !serde_json::to_string(&summary)
            .unwrap()
            .contains("content is never")
    );
    let reopened = Engine::open(EngineConfig {
        data_dir: f.data.path().to_owned(),
        ..EngineConfig::default()
    })
    .unwrap();
    assert_eq!(
        reopened
            .revision_for_job(
                &job.job_id,
                &f.actor,
                &reopened.policy_authorizer().unwrap()
            )
            .unwrap(),
        receipt.revision_id()
    );
    drop(held);
}

#[test]
fn queued_linux_target_replacement_fails_without_receipt_or_latest_change() {
    let f = ExecutionFixture::new();
    let job = f.enqueue();
    std::fs::rename(
        f.source.path().join("target"),
        f.source.path().join("original-retained"),
    )
    .unwrap();
    std::fs::write(f.source.path().join("target"), b"different object").unwrap();
    let result = f.engine.run_job_strict(&job.job_id, "replacement-worker");
    assert!(
        matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
        "actual target mismatch: {result:?}"
    );
    let control = ControlStore::open(&f.data.path().join("diskgraph-control.sqlite")).unwrap();
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Failed);
    assert_eq!(
        control
            .process_job_failure(&job.job_id)
            .unwrap()
            .unwrap()
            .code(),
        ProcessEvidenceFailureCode::Conflict
    );
    assert!(
        f.graph()
            .process_job_publication_receipt(&job.job_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.engine.latest_revision(&f.scope).unwrap().as_deref(),
        Some(f.base.as_str())
    );
}
