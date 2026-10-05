//! 执行请求须能承载真实 Hello 与固定预算终态；仅准入测试，不授予进程或发布资格。
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionFrame, ExecutionOutcome, FrameWriter, ProtocolLimits, WorkerIoKind,
    WorkerRequest,
};
use std::io;

const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";
const MESSAGES: [&str; 6] = [
    "frame byte limit",
    "stream byte limit",
    "node preparation limit",
    "node count limit",
    "sequence or depth limit",
    "declared children exceed node limit",
];

fn ample() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 32768,
        max_nodes: 3,
        max_depth: 0,
    }
}

fn hello() -> ExecutionFrame {
    ExecutionFrame::Hello {
        version: 2,
        target: env!("DISKGRAPH_WORKER_TARGET").into(),
        pin: PIN.into(),
    }
}

fn terminal(message: &str) -> ExecutionFrame {
    ExecutionFrame::Error {
        code: "output_budget".into(),
        io_kind: WorkerIoKind::InvalidData,
        raw_os_error: None,
        message: message.into(),
    }
}

fn encoded(frame: &ExecutionFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    FrameWriter::new(&mut bytes, ample())
        .write_payload(frame)
        .unwrap();
    bytes
}

fn boundary() -> ProtocolLimits {
    let hello = encoded(&hello());
    let error_bytes = MESSAGES
        .iter()
        .map(|message| encoded(&terminal(message)).len())
        .max()
        .unwrap();
    ProtocolLimits {
        max_frame_bytes: (hello.len().max(error_bytes) - 4) as u64,
        max_stream_bytes: (hello.len() + error_bytes) as u64,
        ..ample()
    }
}

fn incoming(root: &std::path::Path, limits: ProtocolLimits) -> WorkerRequest {
    // 模拟真实闭合解码的外来请求，不能借父端 scan 构造器隐藏 into_scan 的缺口。
    let request = WorkerRequest::scan(root, &ScanOptions::default(), ample()).unwrap();
    let mut value = serde_json::to_value(request).unwrap();
    value["limits"]["max_frame_bytes"] = limits.max_frame_bytes.into();
    value["limits"]["max_stream_bytes"] = limits.max_stream_bytes.into();
    value["limits"]["max_nodes"] = limits.max_nodes.into();
    value["limits"]["max_depth"] = limits.max_depth.into();
    serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap()
}

#[test]
fn parent_request_refuses_one_byte_short_combined_hello_and_terminal_allowance() {
    let root = tempfile::tempdir().unwrap();
    let mut limits = boundary();
    limits.max_stream_bytes -= 1;
    let result = WorkerRequest::scan(root.path(), &ScanOptions::default(), limits);
    assert!(
        result.is_err(),
        "unrepresentable Hello plus terminal was admitted"
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn parent_request_refuses_one_byte_short_largest_required_frame() {
    let root = tempfile::tempdir().unwrap();
    let mut limits = boundary();
    limits.max_frame_bytes -= 1;
    let result = WorkerRequest::scan(root.path(), &ScanOptions::default(), limits);
    assert!(
        result.is_err(),
        "required complete frame cannot fit original allowance"
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn decoded_request_rechecks_each_short_allowance_before_scan_preparation() {
    let root = tempfile::tempdir().unwrap();
    for frame_deficit in [false, true] {
        let mut limits = boundary();
        if frame_deficit {
            limits.max_frame_bytes -= 1;
        } else {
            limits.max_stream_bytes -= 1;
        }
        let result = incoming(root.path(), limits).into_scan();
        assert!(
            result.is_err(),
            "wire limits bypassed admission: frame_deficit={frame_deficit}"
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}

#[test]
fn exact_boundary_preserves_original_limits_through_both_request_paths() {
    let root = tempfile::tempdir().unwrap();
    let limits = boundary();
    for request in [
        WorkerRequest::scan(root.path(), &ScanOptions::default(), limits).unwrap(),
        incoming(root.path(), limits),
    ] {
        let (actual_root, _, actual) = request.into_scan().unwrap();
        assert_eq!(actual_root, root.path());
        assert_eq!(actual.max_frame_bytes, limits.max_frame_bytes);
        assert_eq!(actual.max_stream_bytes, limits.max_stream_bytes);
        assert_eq!(actual.max_nodes, limits.max_nodes);
        assert_eq!(actual.max_depth, limits.max_depth);
    }
}

#[test]
fn admitted_boundary_carries_each_real_terminal_on_one_original_ledger() {
    let limits = boundary();
    let root = tempfile::tempdir().unwrap();
    WorkerRequest::scan(root.path(), &ScanOptions::default(), limits).unwrap();
    let mut largest = 0;
    for message in MESSAGES {
        let mut bytes = Vec::new();
        let counted;
        {
            let mut writer = FrameWriter::new(&mut bytes, limits);
            writer.write_payload(&hello()).unwrap();
            writer.write_payload(&terminal(message)).unwrap();
            counted = writer.bytes_written();
        }
        assert_eq!(counted as usize, bytes.len());
        assert!(counted <= limits.max_stream_bytes);
        largest = largest.max(counted);
        let mut decoder =
            ExecutionDecoder::new(limits, env!("DISKGRAPH_WORKER_TARGET"), PIN).unwrap();
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            let (count, _) = decoder.push(remaining).unwrap();
            assert!(count > 0);
            remaining = &remaining[count..];
        }
        assert_eq!(decoder.bytes_admitted(), counted);
        match decoder.finish_eof().unwrap() {
            ExecutionOutcome::Failure(error) => {
                assert_eq!(error.code(), "output_budget");
                assert_eq!(error.io_kind(), io::ErrorKind::InvalidData);
                assert_eq!(error.raw_os_error(), None);
                assert_eq!(error.message(), message);
            }
            ExecutionOutcome::Tree(_) => panic!("terminal budget error became successful tree"),
        }
    }
    assert_eq!(
        largest, limits.max_stream_bytes,
        "the maximum complete pair uses the exact boundary"
    );
}
