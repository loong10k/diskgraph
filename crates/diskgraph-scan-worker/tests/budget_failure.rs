//! 真实输出额度错误的 v2 传递；来源：PF-06 协议额度类型合同。
//! 使用实际 TreeWriter/WorkerFailure/有界帧和解码器，不启动扫描器或推断 OS 退场。

use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionFailure, ExecutionFrame, ExecutionOutcome, FlatNode, FlatNodes,
    Frame, FrameWriter, ProtocolBudgetError, ProtocolLimits, TreeWriter, WorkerFailure,
};
use std::io;

const TARGET: &str = "budget-wire-test";
const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn limits(nodes: u64) -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 65536,
        max_nodes: nodes,
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

fn hello() -> ExecutionFrame {
    ExecutionFrame::Hello {
        version: 2,
        target: TARGET.into(),
        pin: PIN.into(),
    }
}

fn decode(bytes: &[u8], nodes: u64) -> ExecutionOutcome {
    let mut decoder = ExecutionDecoder::new(limits(nodes), TARGET, PIN).unwrap();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let (consumed, _) = decoder.push(remaining).unwrap();
        assert!(consumed > 0 && consumed <= remaining.len());
        remaining = &remaining[consumed..];
    }
    decoder.finish_eof().unwrap()
}

fn roundtrip_failure(error: io::Error) -> ExecutionFailure {
    // Hello 与真实 producer 的 Error 共用一个 FrameWriter 账本，不合成预算错误代码。
    let mut wire = Vec::new();
    let mut writer = FrameWriter::new(&mut wire, limits(1));
    writer.write_payload(&hello()).unwrap();
    writer
        .write_payload(&WorkerFailure::new("output", error))
        .unwrap();
    match decode(&wire, 1) {
        ExecutionOutcome::Failure(failure) => failure,
        ExecutionOutcome::Tree(_) => panic!("output failure became a successful tree"),
    }
}

#[test]
fn actual_output_node_quota_preserves_typed_budget_code_and_original_io_facts() {
    let mut bytes = Vec::new();
    let mut writer = TreeWriter::new(&mut bytes, limits(1));
    let error = writer
        .write_frame(&Frame::Node {
            node: nodes().remove(0),
        })
        .unwrap_err();
    assert_eq!(writer.bytes_written(), 0);
    assert!(error.get_ref().unwrap().is::<ProtocolBudgetError>());
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.raw_os_error(), None);
    let message = error.to_string();
    let failure = roundtrip_failure(error);
    assert_eq!(failure.io_kind(), io::ErrorKind::InvalidData);
    assert_eq!(failure.raw_os_error(), None);
    assert_eq!(failure.message(), message);
    assert_eq!(
        failure.code(),
        "output_budget",
        "actual typed quota must survive WorkerFailure serialization and decoding"
    );
}

#[test]
fn sufficient_node_quota_preserves_complete_tree_fields_and_order() {
    let expected = nodes();
    let mut tree_bytes = Vec::new();
    let mut writer = TreeWriter::new(&mut tree_bytes, limits(2));
    for node in nodes() {
        writer.write_frame(&Frame::Node { node }).unwrap();
    }
    writer.write_frame(&Frame::End { nodes: 2 }).unwrap();
    assert_eq!(writer.finish().unwrap(), 2);
    // 这里只组合有限内存测试流；生产 WorkerOutput 的整流额度不在此重置或替换。
    let mut wire = Vec::new();
    FrameWriter::new(&mut wire, limits(2))
        .write_payload(&hello())
        .unwrap();
    wire.extend(tree_bytes);
    let tree = match decode(&wire, 2) {
        ExecutionOutcome::Tree(tree) => tree,
        ExecutionOutcome::Failure(error) => panic!("unexpected failure: {error:?}"),
    };
    assert_eq!(
        FlatNodes::new(&tree)
            .collect::<io::Result<Vec<_>>>()
            .unwrap(),
        expected
    );
}

#[test]
fn invalid_data_budget_words_cannot_mint_a_budget_code() {
    let message = "node count limit; declared children exceed node limit; output_budget";
    let failure = roundtrip_failure(io::Error::new(io::ErrorKind::InvalidData, message));
    assert_eq!(failure.code(), "output");
    assert_eq!(failure.io_kind(), io::ErrorKind::InvalidData);
    assert_eq!(failure.raw_os_error(), None);
    assert_eq!(failure.message(), message);
}

#[test]
fn real_filesystem_error_preserves_native_facts_without_budget_classification() {
    let directory = tempfile::tempdir().unwrap();
    let error = std::fs::File::open(directory.path().join("missing-file")).unwrap_err();
    let kind = error.kind();
    let raw = error.raw_os_error();
    let message = error.to_string();
    assert!(raw.is_some(), "fixture must observe an actual OS error");
    let failure = roundtrip_failure(error);
    assert_eq!(failure.code(), "output");
    assert_eq!(failure.io_kind(), kind);
    assert_eq!(failure.raw_os_error(), raw);
    assert_eq!(failure.message(), message);
}

#[test]
fn budget_phase_requires_hello_invalid_data_and_absence_of_os_code() {
    for (with_hello, kind, raw) in [
        (false, "invalid_data", None),
        (true, "other", None),
        (true, "invalid_data", Some(2)),
    ] {
        let mut wire = Vec::new();
        let mut writer = FrameWriter::new(&mut wire, limits(1));
        if with_hello {
            writer.write_payload(&hello()).unwrap();
        }
        writer
            .write_payload(&serde_json::json!({
                "type":"error", "code":"output_budget", "io_kind":kind,
                "raw_os_error":raw, "message":"quota words do not validate a forged combination"
            }))
            .unwrap();
        let mut decoder = ExecutionDecoder::new(limits(1), TARGET, PIN).unwrap();
        let mut remaining = wire.as_slice();
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
            "invalid combination must stay terminal"
        );
    }
}
