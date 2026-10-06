//! 用请求局部时钟注入验证真实扫描结束后的发布期限，不依赖机器速度。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::PrincipalId;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn expirations() -> &'static Mutex<HashSet<String>> {
    static JOBS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn publication_elapsed(job_id: &str, elapsed: Duration) -> Duration {
    if expirations().lock().unwrap().remove(job_id) {
        Duration::MAX
    } else {
        elapsed
    }
}

#[test]
fn scan_expiring_after_staging_cannot_publish_a_revision_or_collector() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='late'\nversion='0.1.0'\n",
    )
    .unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("publication-clock").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE staging_seen(node_seq INTEGER); CREATE TRIGGER record_stage AFTER INSERT ON scan_staging BEGIN INSERT INTO staging_seen VALUES(NEW.node_seq); END;").unwrap();
    expirations().lock().unwrap().insert(job.job_id.clone());
    let result = engine.run_job(&job.job_id, "clock-fixture");
    assert!(
        !expirations().lock().unwrap().contains(&job.job_id),
        "did not reach post-staging publication clock"
    );
    let staged: i64 = db
        .query_row("SELECT COUNT(*) FROM staging_seen", [], |row| row.get(0))
        .unwrap();
    assert!(staged > 0, "fixture did not exercise actual staging");
    assert!(
        matches!(
            result,
            Err(EngineError::Store(
                diskgraph_store::StoreError::BudgetExceeded
            ))
        ),
        "post-staging expiration must report budget exhaustion: {result:?}"
    );
    for table in [
        "snapshots",
        "graph_revisions",
        "collector_runs",
        "scan_staging",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "late publication left {table}");
    }
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}
