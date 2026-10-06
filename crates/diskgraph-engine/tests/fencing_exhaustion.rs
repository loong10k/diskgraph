//! 公开 Engine 认领边界：拒绝耗尽代次时不得开始扫描或修改任务。
use diskgraph_core::PrincipalId;
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_store::StoreError;

#[test]
fn exhausted_generation_is_rejected_before_scan_or_publication() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"original").unwrap();
    let data = fixture.path().join("data");
    // 此用例有意不安装扫描宿主：认领拒绝必须发生在任何镜像准入之前。
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("fencing-regression").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let scope = engine.register_scope(&root, &principal, &policy).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &policy).unwrap();
    let connection = rusqlite::Connection::open(data.join("diskgraph-control.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE jobs SET fencing_token=?2 WHERE job_id=?1",
            rusqlite::params![job.job_id, i64::MAX],
        )
        .unwrap();
    let before = engine.job_status(&job.job_id).unwrap();
    let result = engine.run_job(&job.job_id, "new-owner");
    assert!(
        matches!(result, Err(EngineError::Store(StoreError::Conflict(_)))),
        "exhausted claim must reject atomically before scan: {result:?}"
    );
    let after = engine.job_status(&job.job_id).unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.owner, before.owner);
    assert_eq!(after.fencing_token, before.fencing_token);
    assert_eq!(after.heartbeat_unix_ms, before.heartbeat_unix_ms);
    assert_eq!(after.lease_expires_unix_ms, before.lease_expires_unix_ms);
    assert!(engine.latest_revision(&scope).unwrap().is_none());
    assert_eq!(std::fs::read(root.join("file")).unwrap(), b"original");
}
