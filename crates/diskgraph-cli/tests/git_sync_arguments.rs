//! C03 条件参数回归。来源：真实 CLI 解析入口，不以私有 Clap 状态代替进程行为。
use std::process::Command;

fn rejected(arguments: &[&str]) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("unopened-data");
    let output = Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(&data)
        .args(arguments)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "arguments={arguments:?} stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!data.exists(), "无效参数不应打开或创建数据目录");
}

#[test]
fn git_collector_requires_both_revision_and_node_before_opening_the_engine() {
    rejected(&["sync", "--scope", "scope", "--collector", "git"]);
    rejected(&[
        "sync",
        "--scope",
        "scope",
        "--collector",
        "git",
        "--revision",
        "rev",
    ]);
    rejected(&[
        "sync",
        "--scope",
        "scope",
        "--collector",
        "git",
        "--node-id",
        "1",
    ]);
}

#[test]
fn scan_sync_rejects_collector_target_fields_without_a_collector() {
    rejected(&["sync", "--scope", "scope", "--revision", "rev"]);
    rejected(&["sync", "--scope", "scope", "--node-id", "1"]);
    rejected(&[
        "sync",
        "--scope",
        "scope",
        "--revision",
        "rev",
        "--node-id",
        "1",
    ]);
}

#[test]
fn git_sync_rejects_unknown_collectors_and_arbitrary_execution_inputs() {
    rejected(&["sync", "--scope", "scope", "--collector", "process"]);
    for option in [
        "--program",
        "--argv",
        "--path",
        "--max-output-bytes",
        "--network",
    ] {
        rejected(&[
            "sync",
            "--scope",
            "scope",
            "--collector",
            "git",
            "--revision",
            "rev",
            "--node-id",
            "1",
            option,
            "external-input",
        ]);
    }
}

#[test]
fn git_sync_rejects_zero_and_negative_node_identifiers_before_opening_the_engine() {
    for node in ["0", "-1"] {
        rejected(&[
            "sync",
            "--scope",
            "scope",
            "--collector",
            "git",
            "--revision",
            "rev",
            "--node-id",
            node,
        ]);
    }
}
