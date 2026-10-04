//! 进程采集生命周期的真实 Linux 夹具；来源：tmpfs 普通文件、公开索引与持久扫描 epoch。
use crate::{Engine, EngineConfig};
use diskgraph_core::{
    JobRequestAuthority, Permission, PrincipalId, ProcessEvidenceJobInput, ProcessEvidenceLimits,
    ProcessObservationMethod, QueryBudget, QueryReadBudget, ScopeId,
};
use diskgraph_store::JobRecord;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 每例独占源与图库，原身份来自实际 scan；来源：Rust D42 Linux lifecycle 验收。
pub(super) struct ProcessExecutionFixture {
    pub(super) source: tempfile::TempDir,
    pub(super) data: tempfile::TempDir,
    pub(super) engine: Arc<Engine>,
    pub(super) actor: PrincipalId,
    pub(super) scope: ScopeId,
    pub(super) base: String,
    pub(super) node: u64,
}
impl ProcessExecutionFixture {
    pub(super) fn new() -> Self {
        let source = tempfile::tempdir_in("/dev/shm").unwrap();
        let root = source.path().join("container/scope");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("target"), b"private fixture body").unwrap();
        let data = tempfile::tempdir().unwrap();
        let engine = Arc::new(
            Engine::open(EngineConfig {
                data_dir: data.path().to_owned(),
                ..EngineConfig::default()
            })
            .unwrap(),
        );
        let actor = PrincipalId::new("process-lifecycle").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let scope = engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        let auth = engine.policy_authorizer().unwrap();
        let scan = engine.index_scope(&scope, &actor, &auth).unwrap();
        engine.run_job(&scan.job_id, "scan-fixture").unwrap();
        let base = engine
            .revision_for_job(&scan.job_id, &actor, &auth)
            .unwrap();
        let node = engine
            .revision_node_at(&base, Path::new("target"))
            .unwrap()
            .unwrap()
            .id;
        Self {
            source,
            data,
            engine,
            actor,
            scope,
            base,
            node,
        }
    }
    pub(super) fn input(
        &self,
        base: &str,
        limits: ProcessEvidenceLimits,
    ) -> ProcessEvidenceJobInput {
        let graph = self.engine.revision_reader().unwrap();
        let snapshot = graph.revision(base).unwrap().snapshot_id;
        let mut reads = QueryReadBudget::new(
            QueryBudget::default(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let saved = graph
            .unix_observation_bounded(&snapshot, self.node, &mut reads)
            .unwrap()
            .unwrap();
        assert!(
            saved.gap.is_none(),
            "native prerequisite gap: {:?}",
            saved.gap
        );
        ProcessEvidenceJobInput::new(
            self.engine.server_id().unwrap(),
            self.scope.clone(),
            base.to_owned(),
            self.node,
            ProcessObservationMethod::LinuxProcfsV1,
            saved.observation.unwrap().epoch().clone(),
            limits,
        )
        .unwrap()
    }
    pub(super) fn enqueue(&self, base: &str, expiry: u64) -> JobRecord {
        let authority = self.authority(expiry);
        self.engine
            .process_evidence_scope_with_authority(
                &self.scope,
                base,
                self.node,
                &authority,
                &self.engine.policy_authorizer().unwrap(),
            )
            .unwrap()
    }
    pub(super) fn authority(&self, expiry: u64) -> JobRequestAuthority {
        JobRequestAuthority::authenticated_remote(
            self.actor.clone(),
            "fixture-issuer",
            "http",
            vec![Permission::MetadataRead, Permission::IndexWrite],
            expiry,
        )
        .unwrap()
    }
    pub(super) fn assert_no_publication(&self) {
        assert_eq!(
            self.engine.latest_revision(&self.scope).unwrap().as_deref(),
            Some(self.base.as_str())
        );
        let db = rusqlite::Connection::open(self.data.path().join("diskgraph.sqlite")).unwrap();
        for query in [
            "SELECT COUNT(*) FROM collector_runs WHERE collector_id='process-native'",
            "SELECT COUNT(*) FROM process_job_publication_receipts",
            "SELECT COUNT(*) FROM scan_staging",
        ] {
            let count: i64 = db.query_row(query, [], |r| r.get(0)).unwrap();
            assert_eq!(count, 0, "failed publication residue: {query}");
        }
    }
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
