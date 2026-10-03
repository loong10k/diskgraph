//! 实际 CLI 进程验证完整比较报告和 envelope 的字节与截断契约。
use diskgraph_core::QueryBudget;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn run(data: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(data)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn comparison_fits_the_complete_cli_envelope_and_reports_partial() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let root = directory.path().join("root");
    let mut deepest = root.clone();
    for index in 0..3 {
        deepest = deepest.join(format!("{index}{}", "d".repeat(219)));
    }
    std::fs::create_dir_all(&deepest).unwrap();
    for index in 0..90 {
        std::fs::write(deepest.join(format!("{index:03}{}", "f".repeat(197))), b"x").unwrap();
    }
    let added: Value = serde_json::from_slice(&run(
        &data,
        &["scope", "add", "--root", root.to_str().unwrap()],
    ))
    .unwrap();
    let scope = added["data"]["scope_id"].as_str().unwrap();
    run(&data, &["index", "--scope", scope, "--wait"]);
    let bytes = run(
        &data,
        &[
            "compare",
            "--from-scope",
            scope,
            "--to-scope",
            scope,
            "--limit",
            "100",
        ],
    );
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        bytes.len().saturating_sub(1) <= QueryBudget::default().max_response_bytes,
        "actual CLI envelope={} cap={}",
        bytes.len(),
        QueryBudget::default().max_response_bytes
    );
    assert_eq!(reply["data"]["complete"], false);
    assert!(reply["data"]["truncation_reason"].is_string());
    assert_eq!(reply["truncated"], true);
}

#[test]
fn history_decode_failure_keeps_a_bounded_business_error_envelope() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"data").unwrap();
    let added: Value = serde_json::from_slice(&run(
        &data,
        &["scope", "add", "--root", root.to_str().unwrap()],
    ))
    .unwrap();
    let scope = added["data"]["scope_id"].as_str().unwrap();
    let indexed: Value =
        serde_json::from_slice(&run(&data, &["index", "--scope", scope, "--wait"])).unwrap();
    let revision = indexed["data"]["revision_id"].as_str().unwrap();
    // 原始字段仍在读取额度内，实际 JSON 转义后的错误诊断超过默认响应额度。
    rusqlite::Connection::open(data.join("diskgraph.sqlite"))
        .unwrap()
        .execute(
            "UPDATE nodes SET kind=?1 WHERE parent_id IS NULL",
            ["\0".repeat(11_000)],
        )
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(&data)
        .arg("--json")
        .args(["compare", "--from", revision, "--to", revision])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(10));
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["error"]["code"], "internal_error");
    assert!(reply.get("data").is_none());
    assert!(
        output.stdout.len().saturating_sub(1) <= QueryBudget::default().max_response_bytes,
        "encoded failure envelope={} cap={}",
        output.stdout.len(),
        QueryBudget::default().max_response_bytes
    );
}
