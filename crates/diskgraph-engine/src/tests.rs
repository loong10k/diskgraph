use super::EngineConfig;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use diskgraph_core::PrincipalId;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn reclaimed_generation_does_not_reuse_a_cancelled_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), [0]).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..Default::default()
    })
    .unwrap();
    let principal = PrincipalId::new("fixture").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine
        .control_store()
        .unwrap()
        .claim_job(&job.job_id, "old")
        .unwrap();
    engine
        .cancellations()
        .unwrap()
        .insert(job.job_id.clone(), Arc::new(AtomicBool::new(true)));
    rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite"))
        .unwrap()
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&job.job_id],
        )
        .unwrap();
    engine.run_job(&job.job_id, "new").unwrap();
    assert!(engine.latest_revision(&scope).unwrap().is_some());
}
