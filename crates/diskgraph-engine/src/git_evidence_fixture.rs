//! 持久 Git 作业测试的独占原生仓库；来源：实际 Git / Engine 扫描，不伪造索引身份。
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use crate::Engine;
use crate::EngineConfig;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use diskgraph_core::{JobRequestAuthority, Permission, PrincipalId, ScopeId};
use diskgraph_store::JobRecord;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 每例独占源、控制库和图库；来源：原生 Rust EC-02 持久发布回归。
pub(super) struct GitEvidenceFixture {
    pub(super) engine: Arc<Engine>,
    pub(super) temp: tempfile::TempDir,
    pub(super) actor: PrincipalId,
    pub(super) scope: ScopeId,
    pub(super) base: String,
    pub(super) node: u64,
}

impl GitEvidenceFixture {
    pub(super) fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(temp.path().join("empty-config"), "").unwrap();
        let engine = Arc::new(
            Engine::open(EngineConfig {
                data_dir: temp.path().join("data"),
                ..EngineConfig::default()
            })
            .unwrap(),
        );
        let actor = PrincipalId::new("git-job-fixture").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let scope = engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        let mut fixture = Self {
            temp,
            engine,
            actor,
            scope,
            base: String::new(),
            node: 0,
        };
        fixture.git(&["init", "-q"]);
        std::fs::write(root.join("tracked"), "committed\n").unwrap();
        fixture.git(&["add", "tracked"]);
        fixture.git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=f@example.invalid",
            "commit",
            "-qm",
            "base",
        ]);
        let job = fixture
            .engine
            .index_scope(
                &fixture.scope,
                &fixture.actor,
                &fixture.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        fixture.engine.run_job(&job.job_id, "fixture-scan").unwrap();
        fixture.base = fixture
            .engine
            .revision_for_job(
                &job.job_id,
                &fixture.actor,
                &fixture.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        fixture.node = fixture.engine.revision_root_node(&fixture.base).unwrap().id;
        fixture
            .engine
            .set_content_read(&fixture.scope, &fixture.actor, true)
            .unwrap();
        fixture
    }
    pub(super) fn git(&self, arguments: &[&str]) -> Vec<u8> {
        let mut command = Command::new("git");
        command.env_clear();
        for key in ["PATH", "SystemRoot"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let output = command
            .current_dir(self.temp.path().join("repo"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.temp.path().join("empty-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(["-c", "maintenance.auto=false"])
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture Git {arguments:?}: {output:?}"
        );
        output.stdout
    }
    pub(super) fn enqueue(&self, base: &str, expiry: u64) -> JobRecord {
        let authority = JobRequestAuthority::authenticated_remote(
            self.actor.clone(),
            "fixture-issuer",
            "verified-test-adapter",
            vec![
                Permission::MetadataRead,
                Permission::IndexWrite,
                Permission::ContentRead,
            ],
            expiry,
        )
        .unwrap();
        self.engine
            .git_evidence_scope_with_authority(
                &self.scope,
                base,
                self.node,
                &authority,
                &self.engine.policy_authorizer().unwrap(),
            )
            .unwrap()
    }
    pub(super) fn assert_no_git_publication(&self) {
        assert_eq!(
            self.engine.latest_revision(&self.scope).unwrap().as_deref(),
            Some(self.base.as_str())
        );
        let db =
            rusqlite::Connection::open(self.temp.path().join("data/diskgraph.sqlite")).unwrap();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM collector_runs WHERE collector_id='git-local'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let receipts: i64 = db
            .query_row("SELECT COUNT(*) FROM job_publication_receipts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            receipts, 0,
            "failed task must not leave an orphan publication receipt"
        );
    }
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
pub(super) fn wait_until(expiry: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while now() < expiry {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
