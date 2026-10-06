//! CLI 受信扫描配置的启动拒绝回归。来源：PF06 本地宿主三值配置契约。
//! 普通文件只用于验证配置材料；本组不执行 helper，不证明镜像执行或作业回收能力。
use std::ffi::OsStr;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

const CONFIG_KEYS: [&str; 3] = [
    "DISKGRAPH_SCAN_WORKER_PATH",
    "DISKGRAPH_SCAN_WORKER_SHA256",
    "DISKGRAPH_SCAN_WORKER_BYTES",
];
// 固定普通文件字节 b"abc" 的真实 SHA-256；不从邻接清单或被测入口自证摘要。
const IMAGE_DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn ordinary_image(directory: &Path) -> PathBuf {
    let image = directory.join("ordinary-image.bin");
    fs::write(&image, b"abc").unwrap();
    assert!(image.is_absolute());
    image
}

fn doctor_rejects_before_data_creation(data: &Path, configuration: &[(&str, &OsStr)]) {
    assert!(!data.exists(), "入场资格要求 data 目录尚未创建");
    let mut command = Command::new(env!("CARGO_BIN_EXE_diskgraph"));
    command
        .arg("--data-dir")
        .arg(data)
        .args(["--json", "doctor"]);
    // 仅改变此真实子进程的环境，隔离宿主配置，避免污染并行测试。
    for key in CONFIG_KEYS {
        command.env_remove(key);
    }
    for (key, value) in configuration {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.code().is_some() && !output.status.success(),
        "配置必须明确拒绝，不能以信号终止充数：status={:?} data_exists={} stdout={stdout} stderr={stderr}",
        output.status,
        data.exists()
    );
    assert!(
        !data.exists(),
        "配置拒绝必须发生在打开 Engine 前：stdout={stdout} stderr={stderr}"
    );
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["ok"], false, "实际 doctor 失败 envelope：{reply}");
    assert!(reply["data"].is_null(), "启动拒绝不得返回成功数据：{reply}");
    assert_eq!(
        reply["error"]["exit_code"].as_i64(),
        output.status.code().map(i64::from),
        "使用实际 CLI 既有业务退出码：{reply}"
    );
}

#[test]
fn partial_scan_worker_environment_is_rejected_before_engine_open() {
    let workspace = tempfile::tempdir().unwrap();
    let image = ordinary_image(workspace.path());
    let values = [image.as_os_str(), OsStr::new(IMAGE_DIGEST), OsStr::new("3")];
    // 三值的六种非空、不完整组合全部拒绝；每个命令具有独立、未创建的 data 路径。
    for mask in 1_u8..7 {
        let configuration: Vec<_> = CONFIG_KEYS
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1_u8 << *index) != 0)
            .map(|(index, key)| (*key, values[index]))
            .collect();
        doctor_rejects_before_data_creation(
            &workspace.path().join(format!("unopened-data-{mask}")),
            &configuration,
        );
    }
}

#[test]
fn complete_scan_worker_environment_with_invalid_digest_is_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    let image = ordinary_image(workspace.path());
    let invalid_digest = "g".repeat(64);
    doctor_rejects_before_data_creation(
        &workspace.path().join("unopened-data"),
        &[
            (CONFIG_KEYS[0], image.as_os_str()),
            (CONFIG_KEYS[1], OsStr::new(&invalid_digest)),
            (CONFIG_KEYS[2], OsStr::new("3")),
        ],
    );
}

#[test]
fn ordinary_held_image_with_valid_digest_but_wrong_length_is_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    let image = ordinary_image(workspace.path());
    let held_image = File::open(&image).unwrap();
    assert!(held_image.metadata().unwrap().is_file());
    assert_eq!(held_image.metadata().unwrap().len(), 3);
    doctor_rejects_before_data_creation(
        &workspace.path().join("unopened-data"),
        &[
            (CONFIG_KEYS[0], image.as_os_str()),
            (CONFIG_KEYS[1], OsStr::new(IMAGE_DIGEST)),
            (CONFIG_KEYS[2], OsStr::new("4")),
        ],
    );
    // 原普通文件在调用期间真实持有；这不是启动镜像或绑定实际执行身份的正控。
    assert_eq!(held_image.metadata().unwrap().len(), 3);
}

#[cfg(target_os = "macos")]
#[test]
fn ordinary_environment_cannot_replace_root_installed_macos_host() {
    let workspace = tempfile::tempdir().unwrap();
    let image = ordinary_image(workspace.path());
    doctor_rejects_before_data_creation(
        &workspace.path().join("unopened-data"),
        &[
            (CONFIG_KEYS[0], image.as_os_str()),
            (CONFIG_KEYS[1], OsStr::new(IMAGE_DIGEST)),
            (CONFIG_KEYS[2], OsStr::new("3")),
        ],
    );
}
