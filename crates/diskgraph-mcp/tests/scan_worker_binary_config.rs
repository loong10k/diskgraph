//! 真实 MCP 进程启动配置拒绝；来源：PF-06，不以服务构造或 HTTP 200 代替产品入口。
use std::process::{Command, Stdio};

fn startup(values: &[(&str, &str)]) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("must_not_be_created");
    let mut command = Command::new(env!("CARGO_BIN_EXE_diskgraph-mcp"));
    command
        .args(["--transport", "stdio", "--data-dir"])
        .arg(&data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for key in [
        "DISKGRAPH_SCAN_WORKER_PATH",
        "DISKGRAPH_SCAN_WORKER_SHA256",
        "DISKGRAPH_SCAN_WORKER_BYTES",
    ] {
        command.env_remove(key);
    }
    for (key, value) in values {
        command.env(key, value);
    }
    let result = command.output().unwrap();
    assert_eq!(
        result.status.code(),
        Some(10),
        "invalid deployment must fail before protocol serving"
    );
    assert!(
        !data.exists(),
        "invalid deployment must fail before database creation"
    );
}

#[test]
fn partial_configuration_refuses_actual_stdio_startup_before_database_creation() {
    startup(&[("DISKGRAPH_SCAN_WORKER_SHA256", "a")]);
}

#[test]
fn invalid_configuration_refuses_actual_stdio_startup_before_database_creation() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("nonexistent_image");
    startup(&[
        ("DISKGRAPH_SCAN_WORKER_PATH", image.to_str().unwrap()),
        ("DISKGRAPH_SCAN_WORKER_SHA256", "invalid"),
        ("DISKGRAPH_SCAN_WORKER_BYTES", "1"),
    ]);
}

#[test]
fn incorrect_expected_length_refuses_actual_startup_before_database_creation() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("ordinary_material");
    std::fs::write(&image, b"ordinary material").unwrap();
    startup(&[
        ("DISKGRAPH_SCAN_WORKER_PATH", image.to_str().unwrap()),
        ("DISKGRAPH_SCAN_WORKER_SHA256", &"a".repeat(64)),
        ("DISKGRAPH_SCAN_WORKER_BYTES", "1"),
    ]);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn explicit_material_serves_eof_and_finishes_empty_recovery() {
    use sha2::{Digest, Sha256};
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("ordinary_material");
    let bytes = b"held fixture material; not execution proof";
    std::fs::write(&image, bytes).unwrap();
    let data = directory.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_diskgraph-mcp"))
        .args(["--transport", "stdio", "--data-dir"])
        .arg(&data)
        .env("DISKGRAPH_SCAN_WORKER_PATH", &image)
        .env(
            "DISKGRAPH_SCAN_WORKER_SHA256",
            format!("{:x}", Sha256::digest(bytes)),
        )
        .env("DISKGRAPH_SCAN_WORKER_BYTES", bytes.len().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "valid material permits service startup and zero-child shutdown"
    );
    assert!(data.is_dir());
    // 没有请求扫描或创建 OS child；此测试仅证明实际启动/EOF/空恢复路径。
}

#[cfg(target_os = "macos")]
#[test]
fn ordinary_environment_cannot_replace_root_installed_macos_host() {
    use sha2::{Digest, Sha256};
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("ordinary_material");
    let bytes = b"held fixture material; not execution proof";
    std::fs::write(&image, bytes).unwrap();
    startup(&[
        ("DISKGRAPH_SCAN_WORKER_PATH", image.to_str().unwrap()),
        (
            "DISKGRAPH_SCAN_WORKER_SHA256",
            &format!("{:x}", Sha256::digest(bytes)),
        ),
        ("DISKGRAPH_SCAN_WORKER_BYTES", &bytes.len().to_string()),
    ]);
}
