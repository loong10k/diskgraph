//! 原 panic payload 与空恢复责任边界；来源：PF-06，不伪造原生进程 owner。
use super::CliEngineHost;
use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig,
    ScanWorkerRuntimeBudget,
};
use diskgraph_scan_worker::ProtocolLimits;
use std::sync::Arc;

#[test]
fn command_panic_preserves_original_payload_with_external_empty_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("ordinary_image");
    std::fs::write(&image, b"abc").unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(
        [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ],
        3,
    )
    .unwrap();
    let budget = ScanWorkerRuntimeBudget::new(
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
    let host = ScanWorkerHost::new(std::fs::File::open(image).unwrap(), expected, budget).unwrap();
    let (engine, recovery) = Engine::open_with_scan_worker(
        EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        },
        host,
    )
    .unwrap();
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    let engine = Arc::new(engine);
    let survivor = Arc::clone(&engine);
    let session = CliEngineHost {
        engine,
        recovery: Some(recovery),
        #[cfg(windows)]
        probe_recovery: diskgraph_engine::ProbeHost::new(1).unwrap().1,
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.execute(|_| -> Result<(), EngineError> {
            std::panic::panic_any("cli-original-payload")
        })
    }));
    let payload = result.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"cli-original-payload")
    );
    assert!(survivor.server_id().is_ok());
    let root = directory.path().join("scope");
    std::fs::create_dir(&root).unwrap();
    let principal = diskgraph_core::PrincipalId::new("retired-host-test").unwrap();
    survivor.bootstrap_local_admin(&principal).unwrap();
    let auth = survivor.policy_authorizer().unwrap();
    let scope = survivor.register_scope(&root, &principal, &auth).unwrap();
    let job = survivor
        .index_scope(&scope, &principal, &survivor.policy_authorizer().unwrap())
        .unwrap();
    let outcome = survivor.run_job_strict(&job.job_id, "retired-host");
    assert!(
        matches!(
            outcome,
            Err(EngineError::Business(
                diskgraph_core::BusinessError::Conflict
            ))
        ),
        "retired original registry must reject new births before image execution: {outcome:?}"
    );
    assert!(survivor.latest_revision(&scope).unwrap().is_none());
    // 没有 child 出生，只验证同一宿主异常边界，不能代替活跃 child 回收验收。
}
