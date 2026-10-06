//! 真实CLI部署材料的平台准入与空恢复；来源：PF-06，不证明helper执行资格。
use std::process::{Command, Stdio};

#[test]
fn ordinary_image_material_follows_the_platform_admission_contract() {
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
    if cfg!(target_os = "macos") {
        // macOS固定安装契约禁止完整旧环境授予普通路径资格，失败须先于数据库创建。
        assert!(!result.status.success());
        let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"]["code"], "unsupported");
        assert!(!data.exists());
        return;
    }
    assert!(result.status.success());
    let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reply["ok"], true);
    assert!(data.is_dir());
    // doctor 没有启动扫描 child；普通文件及空恢复成功不授予对应平台执行资格。
}
