//! 真正可寻址的长短 Linux 文件定位；来源：公开索引与原生 Unix observation，不导入伪 epoch。
use diskgraph_core::{
    JobRequestAuthority, Permission, PrincipalId, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessObservationMethod, QueryBudget, QueryReadBudget,
};
// 三桌面扫描夹具显式持有受信宿主和原恢复责任，保留原业务断言。
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "../support/native_scan_engine.rs"]
mod native_scan_engine;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use diskgraph_engine::Engine;
use diskgraph_engine::EngineConfig;
use diskgraph_store::{ControlStore, JobRecord, SqliteSnapshotStore};
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use native_scan_engine::NativeScanEngine as Engine;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 实际 tmpfs 源和独占控制/图库；来源：Linux 元数据准备 whole-call 计量夹具。
pub(crate) struct Fixture {
    pub(crate) engine: Engine,
    _source: tempfile::TempDir,
    data: tempfile::TempDir,
    pub(crate) root: PathBuf,
    actor: PrincipalId,
    input: ProcessEvidenceJobInput,
}
impl Fixture {
    pub(crate) fn new(long: bool) -> Self {
        let source = tempfile::tempdir_in("/dev/shm").expect("requires real supported Linux tmpfs");
        let mut root = source.path().join("root");
        if long {
            for i in 0..16 {
                root.push(format!("{i:03}{}", "r".repeat(197)));
            }
        }
        assert!(
            root.as_os_str().len() < 4096 - 32,
            "fixture must remain natively addressable"
        );
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("target"), b"ordinary private target").unwrap();
        let data = tempfile::tempdir().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: data.path().to_owned(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("preparation-measure").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let scope = engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        let auth = engine.policy_authorizer().unwrap();
        let job = engine.index_scope(&scope, &actor, &auth).unwrap();
        engine.run_job(&job.job_id, "fixture-index").unwrap();
        let base = engine.revision_for_job(&job.job_id, &actor, &auth).unwrap();
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
        let saved = graph
            .unix_observation_bounded(&snapshot.id, node.id, &mut reads)
            .unwrap()
            .unwrap();
        assert!(
            saved.gap.is_none(),
            "actual native prerequisites not verified: {:?}",
            saved.gap
        );
        let input = ProcessEvidenceJobInput::new(
            engine.server_id().unwrap(),
            scope,
            base,
            node.id,
            ProcessObservationMethod::LinuxProcfsV1,
            saved.observation.unwrap().epoch().clone(),
            ProcessEvidenceLimits::default(),
        )
        .unwrap();
        Self {
            _source: source,
            data,
            engine,
            root,
            actor,
            input,
        }
    }
    pub(crate) fn enqueue(&self, tiny: bool) -> JobRecord {
        let limits = if tiny {
            ProcessEvidenceLimits::new(15_000, 1, 32768, 65536, 8 << 20, 2, 64).unwrap()
        } else {
            ProcessEvidenceLimits::default()
        };
        let input = ProcessEvidenceJobInput::new(
            self.input.server_id().clone(),
            self.input.scope_id().clone(),
            self.input.base_revision_id().to_owned(),
            self.input.node_id(),
            self.input.method(),
            self.input.indexed_epoch().clone(),
            limits,
        )
        .unwrap();
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60;
        let authority = JobRequestAuthority::authenticated_remote(
            self.actor.clone(),
            "fixture",
            "http",
            vec![Permission::MetadataRead, Permission::IndexWrite],
            expiry,
        )
        .unwrap();
        ControlStore::open(&self.data.path().join("diskgraph-control.sqlite"))
            .unwrap()
            .create_process_evidence_job(&input, &authority, 8)
            .unwrap()
            .unwrap()
    }
    pub(crate) fn assert_failed(&self, id: &str) {
        let control =
            ControlStore::open(&self.data.path().join("diskgraph-control.sqlite")).unwrap();
        assert_eq!(
            control.job(id).unwrap().state,
            diskgraph_store::JobState::Failed
        );
        assert_eq!(
            control.process_job_failure(id).unwrap().unwrap().code(),
            diskgraph_core::ProcessEvidenceFailureCode::BudgetExceeded
        );
        let graph = SqliteSnapshotStore::open(&self.data.path().join("diskgraph.sqlite")).unwrap();
        assert!(graph.process_job_publication_receipt(id).unwrap().is_none());
        assert_eq!(
            self.engine
                .latest_revision(self.input.scope_id())
                .unwrap()
                .as_deref(),
            Some(self.input.base_revision_id())
        );
    }
}
