//! 共享 v2 DTO 与真实 IO producer；来源：现 WorkerRequest、WorkerFailure 的实际 wire。
use diskgraph_disktree_core::{scan::ScanOptions, tree::Metric};
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionEvent, ExecutionOutcome, FrameWriter, NativePath, ProtocolLimits,
    WorkerFailure, WorkerRequest,
};
use std::{io, path::Path};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 32768,
        max_nodes: 32,
        max_depth: 16,
    }
}

#[test]
fn shared_request_preserves_original_root_seven_options_and_explicit_limits() {
    let options = ScanOptions {
        apparent_size: true,
        follow_links: true,
        include_hidden: false,
        one_filesystem: true,
        max_depth: Some(7),
        dedup_hardlinks: false,
        metric: Metric::Files,
    };
    let directory = tempfile::tempdir().unwrap();
    let request = WorkerRequest::scan(directory.path(), &options, limits()).unwrap();
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["type"], "request");
    assert_eq!(value["version"], 2);
    assert_eq!(
        value["request"]["root"],
        serde_json::to_value(NativePath::from_path(directory.path())).unwrap()
    );
    assert_eq!(
        value["request"]["options"],
        serde_json::json!({"apparent_size":true,"follow_links":true,"include_hidden":false,"one_filesystem":true,"max_depth":7,"dedup_hardlinks":false,"metric":1})
    );
    assert_eq!(
        value["limits"],
        serde_json::json!({"max_frame_bytes":8192,"max_stream_bytes":32768,"max_nodes":32,"max_depth":16})
    );
    let (root, decoded, bound) = serde_json::from_value::<WorkerRequest>(value)
        .unwrap()
        .into_scan()
        .unwrap();
    assert_eq!(root, directory.path());
    assert_eq!(
        diskgraph_scan_worker::ScanOptions::from_native(&decoded),
        diskgraph_scan_worker::ScanOptions::from_native(&options)
    );
    assert_eq!(bound.max_stream_bytes, 32768);
}

#[test]
fn request_validation_rejects_unknown_fields_versions_relative_roots_and_limits() {
    for text in [r#"{"type":"cancel","extra":true}"#, r#"{"type":"other"}"#] {
        assert!(serde_json::from_str::<WorkerRequest>(text).is_err());
    }
    let directory = tempfile::tempdir().unwrap();
    let value = serde_json::to_value(
        WorkerRequest::scan(directory.path(), &ScanOptions::default(), limits()).unwrap(),
    )
    .unwrap();
    for field in [
        "principal",
        "database",
        "owner",
        "fence",
        "executable",
        "argv",
    ] {
        let mut forged = value.clone();
        forged[field] = serde_json::json!("forged");
        assert!(serde_json::from_value::<WorkerRequest>(forged).is_err());
    }
    for section in ["request", "limits"] {
        let mut forged = value.clone();
        forged[section]["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<WorkerRequest>(forged).is_err());
    }
    for section in ["options", "root"] {
        let mut forged = value.clone();
        forged["request"][section]["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<WorkerRequest>(forged).is_err());
    }
    let mut wrong_version = value;
    wrong_version["version"] = serde_json::json!(1);
    assert_eq!(
        serde_json::from_value::<WorkerRequest>(wrong_version)
            .unwrap()
            .into_scan()
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(WorkerRequest::scan(Path::new("relative"), &ScanOptions::default(), limits()).is_err());
    let mut zero = limits();
    zero.max_frame_bytes = 0;
    assert!(WorkerRequest::scan(directory.path(), &ScanOptions::default(), zero).is_err());
}

#[test]
fn real_io_failure_wire_retains_kind_os_code_message_and_clean_failure_eof() {
    let actual = io::Error::from_raw_os_error(2);
    let kind = actual.kind();
    let message = actual.to_string();
    let mut wire = Vec::new();
    let mut writer = FrameWriter::new(&mut wire, limits());
    writer
        .write_payload(&WorkerFailure::new("protocol", actual))
        .unwrap();
    let mut input = ExecutionDecoder::new(limits(), "test-target", "expected-pin").unwrap();
    let (consumed, event) = input.push(&wire).unwrap();
    assert_eq!(consumed, wire.len());
    assert!(matches!(event, Some(ExecutionEvent::Failed)));
    match input.finish_eof().unwrap() {
        ExecutionOutcome::Failure(failure) => {
            assert_eq!(failure.code(), "protocol");
            assert_eq!(failure.io_kind(), kind);
            assert_eq!(failure.raw_os_error(), Some(2));
            assert_eq!(failure.message(), message);
        }
        ExecutionOutcome::Tree(_) => panic!("real IO Error became a successful tree"),
    }
}

#[test]
fn execution_error_unknown_kind_field_or_phase_is_not_an_authorization_result() {
    for value in [
        serde_json::json!({"type":"error","code":"protocol","io_kind":"unknown","raw_os_error":null,"message":"x"}),
        serde_json::json!({"type":"error","code":"protocol","io_kind":"other","raw_os_error":null,"message":"x","extra":true}),
        serde_json::json!({"type":"error","code":"forged","io_kind":"other","raw_os_error":null,"message":"x"}),
    ] {
        let body = serde_json::to_vec(&value).unwrap();
        let mut wire = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
        wire.extend(body);
        let mut input = ExecutionDecoder::new(limits(), "test-target", "expected-pin").unwrap();
        assert_eq!(
            input.push(&wire).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(input.finish_eof().is_err());
    }
}
