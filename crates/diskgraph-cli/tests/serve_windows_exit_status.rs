//! Windows serve 的完整退出状态回归。来源：CLI 真实伴随进程 ExitStatus 转交契约。
//! C 夹具只验证退出状态，不实现 MCP，也不证明协议、授权或扫描宿主生命周期。
#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn fixture_artifact(environment: &str) -> PathBuf {
    let artifact = match std::env::var_os(environment) {
        Some(value) => PathBuf::from(value),
        None => panic!("Windows 原生资格缺失：必须显式绑定已编译产物 {environment}"),
    };
    assert!(artifact.is_absolute(), "产物绑定必须为绝对路径");
    assert!(artifact.is_file(), "绑定产物必须是真实文件：{environment}");
    artifact
}

fn assert_serve_preserves_status(environment: &str, expected_bits: u32) {
    let artifact = fixture_artifact(environment);
    // 先执行原始 C 产物，独立证明 ExitProcess 给出了指定的原 32 位状态。
    let direct = Command::new(&artifact).output().unwrap();
    assert_eq!(
        direct.status.code().map(|code| code as u32),
        Some(expected_bits),
        "C 产物资格不符：status={:?} stdout={} stderr={}",
        direct.status,
        String::from_utf8_lossy(&direct.stdout),
        String::from_utf8_lossy(&direct.stderr)
    );
    assert!(!direct.status.success(), "指定状态必须是真实失败");

    let workspace = tempfile::tempdir().unwrap();
    let bundle = workspace.path().join("bundle");
    fs::create_dir(&bundle).unwrap();
    let cli = bundle.join("diskgraph.exe");
    let companion = bundle.join("diskgraph-mcp.exe");
    fs::copy(env!("CARGO_BIN_EXE_diskgraph"), &cli).unwrap();
    fs::copy(&artifact, &companion).unwrap();
    assert_eq!(fs::read(&artifact).unwrap(), fs::read(&companion).unwrap());
    let data = workspace.path().join("unopened-data");
    assert!(!data.exists());

    // 定位依据是实际 current_exe 的相邻文件，不查 PATH，也不调用入口内部替身。
    let output = Command::new(&cli)
        .current_dir(workspace.path())
        .arg("--data-dir")
        .arg(&data)
        .args(["serve", "--transport", "stdio", "--profile", "read-full"])
        .env_remove("DISKGRAPH_SCAN_WORKER_PATH")
        .env_remove("DISKGRAPH_SCAN_WORKER_SHA256")
        .env_remove("DISKGRAPH_SCAN_WORKER_BYTES")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code().map(|code| code as u32),
        Some(expected_bits),
        "CLI 必须保留原 32 位状态：status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "不得把高位失败截断为 success");
    assert!(!data.exists(), "serve 状态转交不得创建 CLI 数据目录");
}

#[test]
fn serve_preserves_c0000000_failure_instead_of_truncating_it_to_success() {
    assert_serve_preserves_status("DISKGRAPH_WINDOWS_EXIT_C0000000_FIXTURE", 0xC000_0000);
}

#[test]
fn serve_preserves_full_c0000005_failure_instead_of_returning_exit_five() {
    assert_serve_preserves_status("DISKGRAPH_WINDOWS_EXIT_C0000005_FIXTURE", 0xC000_0005);
}
