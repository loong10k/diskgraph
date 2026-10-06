//! 真实 CLI 的合法宿主材料及空恢复路径；来源：PF-06，不证明 helper 执行资格。
use std::process::{Command, Stdio};

#[test]
fn valid_material_permits_actual_doctor_and_empty_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("ordinary_image");
    std::fs::write(&image, b"abc").unwrap();
    let data = directory.path().join("data");
    let result = Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(&data)
        .args(["--json", "doctor"])
        .env("DISKGRAPH_SCAN_WORKER_PATH", &image)
        .env(
            "DISKGRAPH_SCAN_WORKER_SHA256",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .env("DISKGRAPH_SCAN_WORKER_BYTES", "3")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(result.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reply["ok"], true);
    assert!(data.is_dir());
    // doctor 没有启动扫描 child；普通文件及空恢复成功不授予对应平台执行资格。
}
