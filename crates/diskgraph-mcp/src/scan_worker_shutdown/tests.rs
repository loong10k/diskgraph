#![cfg(any(target_os = "linux", target_os = "macos"))]
//! 原 MCP 退出函数关闭同一扫描资源池；真实数据库夹具，无 child 出生替身。
use diskgraph_core::{BusinessError, PrincipalId};
use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig,
    ScanWorkerRuntimeBudget,
};
use diskgraph_scan_worker::ProtocolLimits;
use sha2::{Digest, Sha256};

#[test]
fn original_mcp_shutdown_seals_surviving_engine_before_new_birth() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image");
    std::fs::write(&path, b"abc").unwrap();
    let expected =
        ScanWorkerHostConfig::from_expected_image(Sha256::digest(b"abc").into(), 3).unwrap();
    let runtime = ScanWorkerRuntimeBudget::new(
        ProtocolLimits {
            max_frame_bytes: 64 << 10,
            max_stream_bytes: 1 << 20,
            max_nodes: 10,
            max_depth: 8,
        },
        64,
        1,
    )
    .unwrap();
    let host = ScanWorkerHost::new(std::fs::File::open(path).unwrap(), expected, runtime).unwrap();
    let (engine, recovery) = Engine::open_with_scan_worker(
        EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        },
        host,
    )
    .unwrap();
    // 原恢复窗口已经耗尽时，即使资源池为空，也不得作出新的完成确认。
    assert!(
        !super::round(Some(&recovery), std::time::Instant::now()).unwrap(),
        "expired recovery observation must remain unconfirmed"
    );
    super::finish(Some(&recovery));
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    let root = directory.path().join("scope");
    std::fs::create_dir(&root).unwrap();
    let principal = PrincipalId::new("retired-mcp-host").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let outcome = engine.run_job_strict(&job.job_id, "retired-mcp");
    assert!(
        matches!(outcome, Err(EngineError::Business(BusinessError::Conflict))),
        "{outcome:?}"
    );
    assert!(engine.latest_revision(&scope).unwrap().is_none());
    // 原池已关闭，未执行 abc 镜像；此测试不能证明 pending child 或有限退出。
}
