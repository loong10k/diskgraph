//! 扫描原始定位经过暂存、发布与数据库重开的真实回归。

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{BusinessError, PrincipalId, ResourceLocator};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

fn scan(engine: &Engine, root: &Path) -> diskgraph_core::ScopeId {
    let actor = PrincipalId::new("locator-scan").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "locator-fixture").unwrap();
    scope
}

fn raw_native(path: &Path) -> Vec<u8> {
    diskgraph_core::Locator::from_native_path(path)
        .raw_bytes()
        .unwrap()
}

#[test]
fn completed_scan_preserves_native_locator_bytes_and_own_mtime_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("真实-\u{fffd}.txt");
    std::fs::write(&file, "data").unwrap();
    // 自身时间与子树聚合时间不同，避免误把已有聚合列视为自身观测。
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .and_then(|handle| {
            handle.set_times(
                std::fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::from_secs(2_000_000_000)),
            )
        })
        .unwrap();
    let root = root.canonicalize().unwrap();
    let config = EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    };
    let engine = Engine::open(config.clone()).unwrap();
    let scope = scan(&engine, &root);
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    drop(engine);
    let engine = Engine::open(config).unwrap();
    assert_eq!(
        engine.latest_revision(&scope).unwrap().as_deref(),
        Some(revision.as_str())
    );
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    let mut statement = db.prepare(
        "SELECT locator_key,native_locator_encoding,native_locator_raw,self_modified_unix_seconds
         FROM nodes WHERE snapshot_id=(SELECT snapshot_id FROM graph_revisions WHERE revision_id=?1)",
    ).expect("published scan must persist qualified raw locator columns");
    let rows = statement
        .query_map([&revision], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    for (display, encoding, raw, modified) in rows {
        let locator: ResourceLocator = serde_json::from_str(&display).unwrap();
        let ResourceLocator::NativePath(display) = locator else {
            panic!("native scan");
        };
        let path = Path::new(&display);
        assert_eq!(raw, raw_native(path));
        assert_eq!(
            encoding,
            if cfg!(windows) {
                "windows_utf16_le"
            } else {
                "unix_bytes"
            }
        );
        let own = std::fs::metadata(path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert_eq!(modified, Some(own));
    }
}

#[test]
fn scan_staging_budget_includes_raw_locator_storage_cost() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("long-locator-文件.txt"), "data").unwrap();
    let mut config = EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    };
    let observed = diskgraph_disktree::scan_native_v2(&root, config.scan_options.clone()).unwrap();
    let old_cost: u64 = observed
        .nodes
        .iter()
        .map(|node| {
            let ResourceLocator::NativePath(path) = &node.v1.locator else {
                panic!("native scan");
            };
            (serde_json::to_vec(&node.v1).unwrap().len()
                + node.v1.name.to_lowercase().len()
                + path.to_lowercase().len()) as u64
        })
        .sum();
    config.scan_budget.max_staging_bytes = old_cost;
    let engine = Engine::open(config).unwrap();
    let actor = PrincipalId::new("locator-budget").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let result = engine.run_job(&job.job_id, "locator-budget");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "old v1-only quota must reject additional raw locator storage: {result:?}"
    );
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}
