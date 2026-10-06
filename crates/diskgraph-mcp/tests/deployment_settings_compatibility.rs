//! MCP公开部署类型兼容性；来源：PF-06底层Engine统一解析，不代表镜像执行资格。
use std::io::Read;

#[test]
fn mcp_public_deployment_settings_are_the_engine_type() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("worker");
    std::fs::write(&image, b"held fixture").unwrap();
    let settings = diskgraph_mcp::ScanWorkerSettings::from_lookup(|name| match name {
        "DISKGRAPH_SCAN_WORKER_PATH" => Some(image.clone().into_os_string()),
        "DISKGRAPH_SCAN_WORKER_SHA256" => Some("00".repeat(32).into()),
        "DISKGRAPH_SCAN_WORKER_BYTES" => Some("12".into()),
        _ => panic!("unexpected deployment key"),
    })
    .unwrap();
    // 编译时要求同一真实类型，不能由两份解析逻辑或只投影字段的包装器替代。
    let settings: Option<diskgraph_engine::ScanWorkerSettings> = settings;
    let (mut held, _independent_expected) = settings.unwrap().open_held().unwrap();
    let mut bytes = Vec::new();
    held.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"held fixture");
}
