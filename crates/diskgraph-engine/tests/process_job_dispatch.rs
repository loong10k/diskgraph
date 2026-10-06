//! D42 执行类型隔离验收；来源：真实 Engine scan + Control typed job，不伪造原生占用。
//! 外平台 epoch 仅是合法协议输入，故意不可执行；不得把它当作整个 scope 的 Scan。
use diskgraph_core::{
    BusinessError, IndexedFileEpoch, JobRequestAuthority, PrincipalId, ProcessEvidenceFailureCode,
    ProcessEvidenceFailurePhase, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessObservationMethod,
};
// Linux 扫描夹具显式持有受信宿主和原恢复责任，保留原业务断言。
#[cfg(target_os = "linux")]
#[path = "support/native_scan_engine.rs"]
mod native_scan_engine;
#[cfg(not(target_os = "linux"))]
use diskgraph_engine::Engine;
use diskgraph_engine::{EngineConfig, EngineError};
use diskgraph_store::{ControlStore, JobKind, JobState, SqliteSnapshotStore};
#[cfg(target_os = "linux")]
use native_scan_engine::NativeScanEngine as Engine;
use std::path::Path;

#[test]
fn unsupported_process_method_fails_its_own_job_without_a_scope_rescan() {
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("target"), b"indexed original").unwrap();
    let data = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: data.path().to_owned(),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("process-dispatch-fixture").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let auth = engine.policy_authorizer().unwrap();
    let scope = engine.register_scope(source.path(), &actor, &auth).unwrap();
    // 注册在真实控制库授予新 scope 权限，重新读取授权快照后才运行 scan 正控。
    let auth = engine.policy_authorizer().unwrap();
    let scan = engine.index_scope(&scope, &actor, &auth).unwrap();
    engine.run_job(&scan.job_id, "initial-scan").unwrap();
    let revision = engine
        .revision_for_job(&scan.job_id, &actor, &auth)
        .unwrap();
    let node = engine
        .revision_node_at(&revision, Path::new("target"))
        .unwrap()
        .unwrap();
    let initial_snapshot = engine.revision_snapshot(&revision).unwrap();
    // 不在本平台制造 native 证明：外平台方法必须被独立 executor 明确拒绝。
    #[cfg(not(windows))]
    let (method, epoch) = (
        ProcessObservationMethod::WindowsRestartManagerV1,
        IndexedFileEpoch::Windows {
            volume: 1,
            file_id: [1; 16],
            creation_ticks: 1,
        },
    );
    #[cfg(windows)]
    let (method, epoch) = (
        ProcessObservationMethod::LinuxProcfsV1,
        IndexedFileEpoch::LinuxHandle {
            device: 1,
            inode: 1,
            filesystem_domain_sha256: [1; 32],
            handle_type: 1,
            handle_bytes: vec![1; 12],
        },
    );
    let input = ProcessEvidenceJobInput::new(
        engine.server_id().unwrap(),
        scope.clone(),
        revision.clone(),
        node.id,
        method,
        epoch,
        ProcessEvidenceLimits::default(),
    )
    .unwrap();
    let authority = JobRequestAuthority::trusted_local(actor.clone(), "trusted-engine").unwrap();
    let mut control = ControlStore::open(&data.path().join("diskgraph-control.sqlite")).unwrap();
    let job = control
        .create_process_evidence_job(&input, &authority, 8)
        .unwrap()
        .unwrap();
    assert_eq!(job.kind, JobKind::ProcessEvidence);
    // 原始范围现在可观察地变化，误入 Scan 就会发布一个不同 snapshot。
    std::fs::write(
        source.path().join("must-not-be-scanned"),
        b"new scan material",
    )
    .unwrap();
    let before = graph_counts(data.path());
    let result = engine.run_job_strict(&job.job_id, "process-worker");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::Unsupported))
        ),
        "actual outcome: {result:?}"
    );
    assert_eq!(control.job(&job.job_id).unwrap().state, JobState::Failed);
    let failure = control.process_job_failure(&job.job_id).unwrap().unwrap();
    assert_eq!(failure.phase(), ProcessEvidenceFailurePhase::Admission);
    assert_eq!(failure.code(), ProcessEvidenceFailureCode::Unsupported);
    assert_eq!(
        engine.latest_revision(&scope).unwrap().as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(
        engine.revision_snapshot(&revision).unwrap().id,
        initial_snapshot.id
    );
    assert_eq!(
        graph_counts(data.path()),
        before,
        "no snapshot/revision/run/staging may leak from wrong dispatch"
    );
    let graph = SqliteSnapshotStore::open(&data.path().join("diskgraph.sqlite")).unwrap();
    assert!(
        graph
            .process_job_publication_receipt(&job.job_id)
            .unwrap()
            .is_none()
    );
    assert!(
        graph
            .job_publication_receipt(&job.job_id)
            .unwrap()
            .is_none()
    );
}

fn graph_counts(data: &Path) -> Vec<i64> {
    let db = rusqlite::Connection::open(data.join("diskgraph.sqlite")).unwrap();
    [
        "snapshots",
        "graph_revisions",
        "collector_runs",
        "scan_staging",
    ]
    .iter()
    .map(|name| {
        db.query_row(&format!("SELECT COUNT(*) FROM {name}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    })
    .collect()
}
