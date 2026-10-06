//! 父端真实节点配额与协议错误分类；来源：OpenSpec 15.2 / PF-06 原预算类型合同。
//! 仅证明实际帧解码、组装与错误投影，不建立 child、执行镜像或 OS 退出许可。

use crate::EngineError;
use crate::scan_worker_error_projection::ScanWorkerErrorProjection;
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_output::ScanWorkerOutput;
use diskgraph_core::BusinessError;
use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionFailure, ExecutionFrame, ExecutionOutcome, FlatNode, FlatNodes,
    Frame, FrameWriter, ProtocolLimits, TreeWriter, WorkerFailure,
};
use std::convert::Infallible;
use std::io;

const TARGET: &str = "budget-classification-fixture";
const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn limits(max_nodes: u64) -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 65536,
        max_nodes,
        max_depth: 8,
    }
}

fn nodes() -> Vec<FlatNode> {
    let mut root = Node::directory("/observed-root");
    root.children.push(Node::entry("file", NodeKind::File, 17));
    FlatNodes::new(&root)
        .collect::<io::Result<Vec<_>>>()
        .unwrap()
}

fn encoded(nodes: Vec<FlatNode>) -> Vec<u8> {
    let count = nodes.len() as u64;
    let mut bytes = Vec::new();
    let mut writer = FrameWriter::new(&mut bytes, limits(2));
    writer
        .write_payload(&ExecutionFrame::<String, String>::Hello {
            version: 2,
            target: TARGET.into(),
            pin: PIN.into(),
        })
        .unwrap();
    for node in nodes {
        writer
            .write_payload(&ExecutionFrame::<String, String>::Node { node })
            .unwrap();
    }
    writer
        .write_payload(&ExecutionFrame::<String, String>::End { nodes: count })
        .unwrap();
    bytes
}

fn check() -> Result<(), ScanWorkerFailure<Infallible>> {
    Ok(())
}

fn project(error: ScanWorkerFailure<Infallible>) -> EngineError {
    ScanWorkerErrorProjection::driver(error, |never| match never {})
}

fn assert_protocol(error: EngineError) {
    assert!(
        matches!(&error, EngineError::Io(source) if source.kind() == io::ErrorKind::InvalidData),
        "malformed protocol must remain InvalidData, got {error:?}"
    );
}

#[test]
fn valid_two_node_stream_over_original_node_quota_is_business_budget() {
    let bytes = encoded(nodes());
    let mut output = ScanWorkerOutput::new(limits(1), (TARGET, PIN), 0).unwrap();
    // 字节/深度足额，唯一差异为原接收端节点限额；错误由真实 TreeState 产生。
    let failure = output.stdout(&bytes, &mut check).unwrap_err();
    assert!(!output.terminal());
    assert!(output.take().is_none());
    let error = project(failure);
    assert!(
        matches!(&error, EngineError::Business(BusinessError::BudgetExceeded)),
        "actual node quota must preserve Business BudgetExceeded, got {error:?}"
    );
}

#[test]
fn same_stream_with_sufficient_quota_assembles_exact_native_fields_and_order() {
    let expected = nodes();
    let bytes = encoded(nodes());
    let mut output = ScanWorkerOutput::new(limits(2), (TARGET, PIN), 0).unwrap();
    // 分片是实际字节输入，不替换解码器结果；本内存源完整结束后才调用 finish。
    for part in bytes.chunks(7) {
        output.stdout(part, &mut check).unwrap();
    }
    assert!(output.terminal());
    assert!(!output.tree(), "End alone has not assembled the tree");
    output.finish(&mut check).unwrap();
    assert!(output.tree());
    let tree = match output.take().unwrap() {
        ExecutionOutcome::Tree(tree) => tree,
        ExecutionOutcome::Failure(error) => panic!("unexpected failure: {error:?}"),
    };
    let actual = FlatNodes::new(&tree)
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 2);
    assert!(output.take().is_none());
}

#[test]
fn wrong_parent_and_duplicate_sequence_remain_protocol_errors_with_sufficient_quota() {
    for wrong_parent in [true, false] {
        let mut records = nodes();
        if wrong_parent {
            records[1].parent = Some(1);
        } else {
            records[1].sequence = 0;
        }
        let bytes = encoded(records);
        let mut output = ScanWorkerOutput::new(limits(2), (TARGET, PIN), 0).unwrap();
        let failure = output.stdout(&bytes, &mut check).unwrap_err();
        assert!(!output.terminal());
        assert!(output.take().is_none());
        assert_protocol(project(failure));
    }
}

#[test]
fn ordinary_invalid_data_containing_budget_words_is_not_a_budget_witness() {
    let original = io::Error::new(
        io::ErrorKind::InvalidData,
        "node count limit; declared children exceed node limit; BudgetExceeded",
    );
    let error = project(ScanWorkerFailure::from(original));
    assert_protocol(error);
}

fn remote_hello() -> ExecutionFrame {
    ExecutionFrame::Hello {
        version: 2,
        target: TARGET.into(),
        pin: PIN.into(),
    }
}

fn checked_remote_failure(error: io::Error) -> ExecutionFailure {
    let mut bytes = Vec::new();
    let mut writer = FrameWriter::new(&mut bytes, limits(1));
    writer.write_payload(&remote_hello()).unwrap();
    writer
        .write_payload(&WorkerFailure::new("output", error))
        .unwrap();
    let mut decoder = ExecutionDecoder::new(limits(1), TARGET, PIN).unwrap();
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let (consumed, _) = decoder.push(remaining).unwrap();
        assert!(consumed > 0 && consumed <= remaining.len());
        remaining = &remaining[consumed..];
    }
    match decoder.finish_eof().unwrap() {
        ExecutionOutcome::Failure(failure) => failure,
        ExecutionOutcome::Tree(_) => panic!("actual failure must not deliver a tree"),
    }
}

#[test]
fn actual_remote_output_quota_reaches_runtime_central_business_budget_projection() {
    let mut bytes = Vec::new();
    let mut writer = TreeWriter::new(&mut bytes, limits(1));
    let error = writer
        .write_frame(&Frame::Node {
            node: nodes().remove(0),
        })
        .unwrap_err();
    assert_eq!(writer.bytes_written(), 0);
    let message = error.to_string();
    let failure = checked_remote_failure(error);
    assert_eq!(failure.code(), "output_budget");
    assert_eq!(failure.io_kind(), io::ErrorKind::InvalidData);
    assert_eq!(failure.raw_os_error(), None);
    assert_eq!(failure.message(), message);
    // 与 runtime 的真实 Failure 分支调用相同入口，不经全局 IO 转换猜预算。
    let result = ScanWorkerErrorProjection::remote(failure);
    assert!(
        matches!(
            &result,
            EngineError::Business(BusinessError::BudgetExceeded)
        ),
        "actual remote quota lost its business category: {result:?}"
    );
}

#[test]
fn remote_invalid_data_budget_words_remain_original_io_diagnostic() {
    let message = "output_budget; node count limit; BudgetExceeded";
    let failure = checked_remote_failure(io::Error::new(io::ErrorKind::InvalidData, message));
    assert_eq!(failure.code(), "output");
    assert_eq!(failure.message(), message);
    assert_eq!(failure.raw_os_error(), None);
    let result = ScanWorkerErrorProjection::remote(failure);
    let source = match result {
        EngineError::Io(source) => source,
        error => panic!("ordinary InvalidData changed category: {error:?}"),
    };
    assert_eq!(source.kind(), io::ErrorKind::InvalidData);
    assert_eq!(source.to_string(), format!("scan worker output: {message}"));
    assert!(source.get_ref().unwrap().source().is_none());
}

#[test]
fn remote_real_filesystem_error_retains_native_kind_code_and_message() {
    let directory = tempfile::tempdir().unwrap();
    let original = std::fs::File::open(directory.path().join("absent-file")).unwrap_err();
    let kind = original.kind();
    let raw = original.raw_os_error();
    let message = original.to_string();
    assert!(raw.is_some(), "fixture requires a real OS failure");
    let failure = checked_remote_failure(original);
    assert_eq!(failure.code(), "output");
    assert_eq!(failure.io_kind(), kind);
    assert_eq!(failure.raw_os_error(), raw);
    assert_eq!(failure.message(), message);
    let result = ScanWorkerErrorProjection::remote(failure);
    let source = match result {
        EngineError::Io(source) => source,
        error => panic!("native failure changed category: {error:?}"),
    };
    assert_eq!(source.kind(), kind);
    assert_eq!(source.to_string(), format!("scan worker output: {message}"));
    // 外层 IO 拥有远端事实；真实 OS 码保存在它的原生 source，不假称跨进程对象相同。
    let native = source
        .get_ref()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(native.raw_os_error(), raw);
    assert_eq!(native.kind(), kind);
}

#[test]
fn remote_budget_forged_fields_and_prehello_order_never_reach_projection() {
    for (with_hello, kind, raw) in [
        (false, "invalid_data", None),
        (true, "other", None),
        (true, "invalid_data", Some(2)),
    ] {
        let mut bytes = Vec::new();
        let mut writer = FrameWriter::new(&mut bytes, limits(1));
        if with_hello {
            writer.write_payload(&remote_hello()).unwrap();
        }
        writer
            .write_payload(&serde_json::json!({
                "type":"error", "code":"output_budget", "io_kind":kind,
                "raw_os_error":raw, "message":"must not become an accepted failure"
            }))
            .unwrap();
        let mut decoder = ExecutionDecoder::new(limits(1), TARGET, PIN).unwrap();
        let mut remaining = bytes.as_slice();
        if with_hello {
            let (consumed, _) = decoder.push(remaining).unwrap();
            remaining = &remaining[consumed..];
        }
        assert_eq!(
            decoder.push(remaining).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(
            decoder.finish_eof().is_err(),
            "invalid failure must stay latched"
        );
    }
}
