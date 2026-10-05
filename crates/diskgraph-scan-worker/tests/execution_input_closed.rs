//! 现有执行 wire 的真实 helper 资格；来源：PF-06 与 WorkerRequest 的闭合派生解码。
//! 这些测试不依赖新解码接口，也不将正常树/EOF 当作进程组退出许可。
mod worker_process {
    pub(super) mod worker_output;
}

use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{NativePath, ScanOptions as WireOptions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use worker_process::worker_output::WorkerOutput;

fn binary() -> PathBuf {
    let artifact = match option_env!("CARGO_BIN_EXE_diskgraph-scan-worker") {
        Some(value) => value,
        None => panic!("artifact qualification requires the actual Cargo worker binary"),
    };
    let binary = PathBuf::from(artifact);
    assert!(binary.is_absolute() && binary.is_file());
    binary
}

fn request(root: &Path) -> Value {
    json!({"type":"request","version":2,
        "request":{"root":NativePath::from_path(root),
            "options":WireOptions::from_native(&ScanOptions::default())},
        "limits":{"max_frame_bytes":8192,"max_stream_bytes":131072,
            "max_nodes":32,"max_depth":16}})
}

fn run(value: &Value, root: &Path) -> WorkerOutput {
    let body = serde_json::to_vec(value).unwrap();
    let mut packet = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    packet.extend(body);
    WorkerOutput::run(&binary(), root, &packet)
}

fn assert_protocol_failure(value: Value) {
    let directory = tempfile::tempdir().unwrap();
    let output = run(&value, directory.path());
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout_bytes > 0);
    assert_eq!(output.frames.len(), 1, "prepare failure has no Hello/End");
    let failure = &output.frames[0];
    assert_eq!(failure["type"], "error");
    assert_eq!(failure["code"], "protocol");
    assert_eq!(failure["io_kind"], "invalid_data");
    assert!(
        failure["message"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
}

#[test]
fn real_request_unknown_outer_fields_fail_before_hello() {
    let directory = tempfile::tempdir().unwrap();
    for field in [
        "principal",
        "database",
        "owner",
        "fence",
        "executable",
        "argv",
    ] {
        let mut value = request(directory.path());
        value[field] = json!("forged");
        assert_protocol_failure(value);
    }
}

#[test]
fn real_request_nested_options_unknown_field_is_closed() {
    let directory = tempfile::tempdir().unwrap();
    let mut value = request(directory.path());
    value["request"]["options"]["extra"] = json!(true);
    assert_protocol_failure(value);
}

#[test]
fn real_request_nested_path_unknown_field_is_closed() {
    let directory = tempfile::tempdir().unwrap();
    let mut value = request(directory.path());
    value["request"]["root"]["extra"] = json!(true);
    assert_protocol_failure(value);
}

#[test]
fn real_request_nested_limits_and_request_unknown_fields_are_closed() {
    let directory = tempfile::tempdir().unwrap();
    for section in ["limits", "request"] {
        let mut value = request(directory.path());
        value[section]["extra"] = json!(true);
        assert_protocol_failure(value);
    }
}

#[test]
fn real_request_preserves_all_seven_options_and_explicit_limits() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("file"), b"native input").unwrap();
    let mut value = request(directory.path());
    value["request"]["options"] = json!({"apparent_size":true,"follow_links":false,
        "include_hidden":true,"one_filesystem":false,"max_depth":7,
        "dedup_hardlinks":false,"metric":1});
    let output = run(&value, directory.path());
    assert!(output.status.success(), "{:?}", output.frames);
    assert!(output.stderr.is_empty());
    assert_eq!(output.frames.first().unwrap()["type"], "hello");
    assert_eq!(output.frames.first().unwrap()["version"], 2);
    assert_eq!(output.frames.last().unwrap()["type"], "end");
    assert_eq!(output.frames.last().unwrap()["nodes"], 2);
    assert!(
        output
            .frames
            .iter()
            .any(|frame| frame["type"] == "progress")
    );
    assert!(!output.frames.iter().any(|frame| frame["type"] == "error"));
}
