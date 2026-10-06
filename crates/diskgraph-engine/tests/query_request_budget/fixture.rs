use diskgraph_core::{Locator, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
// 三桌面扫描夹具显式持有受信宿主和原恢复责任，保留原业务断言。
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "../support/native_scan_engine.rs"]
mod native_scan_engine;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use diskgraph_engine::Engine;
use diskgraph_engine::EngineConfig;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use native_scan_engine::NativeScanEngine as Engine;
use rusqlite::Connection;

/// D24 的独占数据库与真实扫描夹具；来源：Q-02/08 请求预算验收。
pub(crate) struct Fixture {
    pub(crate) engine: Engine,
    pub(crate) directory: tempfile::TempDir,
    pub(crate) principal: PrincipalId,
    pub(crate) scope: ScopeId,
    pub(crate) revision: String,
    pub(crate) snapshot: String,
    pub(crate) db: Connection,
    pub(crate) policy: PolicyAuthorizer,
}

impl Fixture {
    pub(crate) fn new(persistent_policy: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        for name in ["a", "b"] {
            std::fs::write(root.join(name), b"12345678").unwrap();
        }
        let root = root.canonicalize().unwrap();
        let engine = Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new("query-request-budget").unwrap();
        let scope = if persistent_policy {
            engine.bootstrap_local_admin(&principal).unwrap();
            engine
                .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
                .unwrap()
        } else {
            engine
                .control_store()
                .unwrap()
                .register_scope(&Locator::from_native_path(&root), None)
                .unwrap()
        };
        let policy = if persistent_policy {
            engine.policy_authorizer().unwrap()
        } else {
            let mut policy = PolicyAuthorizer::new(1);
            for permission in [Permission::IndexWrite, Permission::MetadataRead] {
                policy.grant(principal.clone(), permission, scope.clone());
            }
            policy
        };
        let job = engine.index_scope(&scope, &principal, &policy).unwrap();
        engine.run_job(&job.job_id, "query-budget-owner").unwrap();
        let revision = engine.latest_revision(&scope).unwrap().unwrap();
        let snapshot = engine
            .revision_reader()
            .unwrap()
            .revision(&revision)
            .unwrap()
            .snapshot_id;
        let db = Connection::open(directory.path().join("data/diskgraph.sqlite")).unwrap();
        Self {
            directory,
            engine,
            principal,
            scope,
            revision,
            snapshot,
            db,
            policy,
        }
    }
}
