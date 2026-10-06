//! 显式受信宿主材料、独立传输额度与 recovery 生命周期入口合同；来源：PF-06。
//! 普通文件仅用于持有/配置资格，未执行它，不把配置或零槽 drain 当 OS 回收证明。

use diskgraph_core::BusinessError;
use diskgraph_engine::{
    Engine, EngineConfig, EngineError, ScanWorkerHost, ScanWorkerHostConfig,
    ScanWorkerRuntimeBudget,
};
use diskgraph_scan_worker::ProtocolLimits;
use sha2::{Digest, Sha256};
use std::fs::File;

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 64 << 10,
        max_stream_bytes: 1 << 20,
        max_nodes: 100,
        max_depth: 8,
    }
}

fn invalid<T>(result: Result<T, EngineError>) {
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::InvalidArgument))
    ));
}

#[test]
fn host_response_and_capacity_are_explicit_and_preserve_wire_values() {
    for field in 0..3 {
        let mut value = limits();
        match field {
            0 => value.max_frame_bytes = 0,
            1 => value.max_stream_bytes = 3,
            _ => value.max_nodes = 0,
        }
        invalid(ScanWorkerRuntimeBudget::new(value, 0, 1));
    }
    invalid(ScanWorkerRuntimeBudget::new(limits(), 0, 0));
    invalid(ScanWorkerRuntimeBudget::new(limits(), (1 << 20) + 1, 1));
    let budget = ScanWorkerRuntimeBudget::new(limits(), 123, 2).unwrap();
    let actual = budget.response_limits();
    assert_eq!(actual.max_frame_bytes, 64 << 10);
    assert_eq!(actual.max_stream_bytes, 1 << 20);
    assert_eq!(actual.max_nodes, 100);
    assert_eq!(actual.max_depth, 8);
    assert_eq!(budget.stderr_bytes(), 123);
    assert_eq!(budget.max_active_children(), 2);
    // 深度0原协议表示仅根；stderr0表示不接受stderr。二者不被新宿主擅自扩大。
    let mut root_only = limits();
    root_only.max_depth = 0;
    let root_only = ScanWorkerRuntimeBudget::new(root_only, 0, 1).unwrap();
    assert_eq!(root_only.response_limits().max_depth, 0);
    assert_eq!(root_only.stderr_bytes(), 0);
}

#[test]
fn explicit_held_material_opens_both_stores_and_returns_external_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let image_path = directory.path().join("held-installation-material");
    let bytes = b"real ordinary file, not an execution proof";
    std::fs::write(&image_path, bytes).unwrap();
    let held = File::open(&image_path).unwrap();
    let expected =
        ScanWorkerHostConfig::from_expected_image(Sha256::digest(bytes).into(), bytes.len() as u64)
            .unwrap();
    let budget = ScanWorkerRuntimeBudget::new(limits(), 64 << 10, 1).unwrap();
    let host = ScanWorkerHost::new(held, expected, budget).unwrap();
    // 路径旁的未受信清单不参与配置；host使用已持有原File及独立expected。
    std::fs::write(
        directory.path().join("worker-manifest.json"),
        b"untrusted neighboring metadata",
    )
    .unwrap();
    let config = EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    };
    let preserved = config.clone();
    let (engine, recovery) = Engine::open_with_scan_worker(config, host).unwrap();
    assert_eq!(engine.data_dir(), preserved.data_dir);
    let server = engine.server_id().unwrap();
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(
        recovery.drain().unwrap(),
        "no child was born, so there is nothing to wait"
    );
    // 服务先释放后，独立Recovery仍可明确处置；不会凭Engine Drop清空failed slot。
    drop(engine);
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(recovery.drain().unwrap());
    let reopened = Engine::open(preserved).unwrap();
    assert_eq!(reopened.server_id().unwrap(), server);
}
