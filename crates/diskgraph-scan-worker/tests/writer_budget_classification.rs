//! PF-06 真实树写入准备及字节额度分类；来源：原生 Rust 有界 codec 公共入口。
//! 不证明 helper 在写端锁存后仍能发送 Error，也不改变既有分配计量门禁。
use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{
    FlatNode, FlatNodes, Frame, FrameWriter, ProtocolBudgetError, ProtocolLimits, read_tree,
    write_frame, write_tree_with_limits,
};
use std::io::{self, Write};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 32768,
        max_nodes: 8,
        max_depth: 4,
    }
}

fn tree(name: &str) -> Node {
    let mut root = Node::directory("/root");
    root.children.push(Node::entry(name, NodeKind::File, 19));
    root
}

fn encoded_node(node: &Node, sequence: u64, parent: Option<u64>, depth: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_frame(
        &mut bytes,
        &Frame::Node {
            node: FlatNode::from_native(node, sequence, parent, depth),
        },
    )
    .unwrap();
    bytes
}

fn assert_budget(error: &io::Error, message: &str) {
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), message);
    assert!(
        error
            .get_ref()
            .is_some_and(|cause| cause.is::<ProtocolBudgetError>()),
        "actual bounded producer lost its typed quota cause: {error:?}"
    );
}

fn refusal(root: &Node, bound: ProtocolLimits, prefix: &[u8], message: &str) {
    let mut output = Vec::new();
    let error = write_tree_with_limits(&mut output, root, bound).unwrap_err();
    assert_eq!(output, prefix, "offending frame must not reach the sink");
    assert_budget(&error, message);
}

#[test]
fn zero_nodes_preparation_is_typed_before_any_frame() {
    let mut bound = limits();
    bound.max_nodes = 0;
    refusal(
        &Node::directory("/root"),
        bound,
        &[],
        "node preparation limit",
    );
}

#[test]
fn child_depth_preparation_is_typed_after_real_root_prefix() {
    let root = tree("file");
    let prefix = encoded_node(&root, 0, None, 0);
    let mut bound = limits();
    bound.max_depth = 0;
    refusal(&root, bound, &prefix, "node preparation limit");
}

#[test]
fn name_lower_bound_preparation_is_typed_before_copy_or_frame() {
    let root = Node::directory("r".repeat(1024));
    let mut bound = limits();
    bound.max_frame_bytes = 32;
    refusal(&root, bound, &[], "node preparation limit");
}

#[test]
fn actual_json_escaping_frame_quota_keeps_typed_cause() {
    let root = tree(&"\"".repeat(512));
    let prefix = encoded_node(&root, 0, None, 0);
    let child = encoded_node(&root.children[0], 1, Some(0), 1);
    let mut bound = limits();
    bound.max_frame_bytes = child.len() as u64 - 4 - 1;
    assert!(bound.max_frame_bytes > root.children[0].name.len() as u64);
    assert!(bound.max_frame_bytes >= prefix.len() as u64 - 4);
    refusal(&root, bound, &prefix, "frame byte limit");
}

#[test]
fn cumulative_stream_body_including_headers_keeps_typed_cause() {
    let root = tree("file");
    let prefix = encoded_node(&root, 0, None, 0);
    let child = encoded_node(&root.children[0], 1, Some(0), 1);
    let mut bound = limits();
    bound.max_stream_bytes = (prefix.len() + child.len() - 1) as u64;
    refusal(&root, bound, &prefix, "frame byte limit");
}

#[test]
fn cumulative_stream_cannot_omit_next_four_byte_header() {
    let root = tree("file");
    let prefix = encoded_node(&root, 0, None, 0);
    let mut bound = limits();
    bound.max_stream_bytes = prefix.len() as u64 + 3;
    refusal(&root, bound, &prefix, "stream byte limit");
}

#[test]
fn exact_complete_tree_budget_preserves_every_field_and_order() {
    let root = tree(&"\"".repeat(512));
    let mut expected = Vec::new();
    assert_eq!(
        write_tree_with_limits(&mut expected, &root, limits()).unwrap(),
        2
    );
    let mut bound = limits();
    bound.max_stream_bytes = expected.len() as u64;
    let mut actual = Vec::new();
    assert_eq!(
        write_tree_with_limits(&mut actual, &root, bound).unwrap(),
        2
    );
    assert_eq!(actual, expected);
    let decoded = read_tree(actual.as_slice(), bound).unwrap();
    let before: Vec<_> = FlatNodes::new(&root).collect::<Result<_, _>>().unwrap();
    let after: Vec<_> = FlatNodes::new(&decoded).collect::<Result<_, _>>().unwrap();
    assert_eq!(before, after);
}

#[test]
fn typed_serialization_failure_stays_latched_without_emergency_error_frame() {
    let mut bound = limits();
    bound.max_frame_bytes = 32;
    let mut output = Vec::new();
    let first;
    {
        let mut writer = FrameWriter::new(&mut output, bound);
        first = writer
            .write_frame(&Frame::Error {
                code: "output".into(),
                message: "x".repeat(128),
            })
            .unwrap_err();
        let second = writer.write_frame(&Frame::Cancel).unwrap_err();
        assert_eq!(second.kind(), io::ErrorKind::Other);
        assert_eq!(second.to_string(), "frame writer already failed");
        assert_eq!(writer.bytes_written(), 0);
    }
    assert!(output.is_empty());
    assert_budget(&first, "frame byte limit");
}

#[test]
fn actual_read_only_sink_error_keeps_os_kind_and_errno_not_quota() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut direct = std::fs::File::open(file.path()).unwrap();
    let expected = direct.write_all(b"qualification").unwrap_err();
    let mut sink = std::fs::File::open(file.path()).unwrap();
    let actual = write_tree_with_limits(&mut sink, &tree("file"), limits()).unwrap_err();
    assert_eq!(actual.kind(), expected.kind());
    assert_eq!(actual.raw_os_error(), expected.raw_os_error());
    assert_eq!(actual.to_string(), expected.to_string());
    assert!(
        !actual
            .get_ref()
            .is_some_and(|cause| cause.is::<ProtocolBudgetError>())
    );
}

/// 合法自定义 Serialize 的错误传播对照；来源：serde 公共 SerializeSeq/Serializer 接口。
enum SerializationProbe {
    SwallowSinkQuota,
    OrdinaryCustomError,
}

impl serde::Serialize for SerializationProbe {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error, SerializeSeq};
        match self {
            Self::SwallowSinkQuota => {
                let mut sequence = serializer.serialize_seq(Some(1))?;
                let _ = sequence.serialize_element(&"x".repeat(1024));
                sequence.end()
            }
            Self::OrdinaryCustomError => Err(S::Error::custom("frame byte limit")),
        }
    }
}

#[test]
fn serializer_cannot_swallow_real_sink_quota_and_publish_partial_json() {
    let mut bound = limits();
    bound.max_frame_bytes = 4;
    let mut output = Vec::new();
    let (first, second, written);
    {
        let mut writer = FrameWriter::new(&mut output, bound);
        first = writer.write_payload(&SerializationProbe::SwallowSinkQuota);
        written = writer.bytes_written();
        second = writer.write_frame(&Frame::Cancel);
    }
    assert!(
        output.is_empty(),
        "swallowed quota emitted a malformed frame: {output:?}"
    );
    assert_eq!(written, 0);
    assert_budget(&first.unwrap_err(), "frame byte limit");
    let second = second.unwrap_err();
    assert_eq!(second.kind(), io::ErrorKind::Other);
    assert_eq!(second.to_string(), "frame writer already failed");
}

#[test]
fn ordinary_custom_serialization_error_with_quota_text_is_not_typed_budget() {
    let mut output = Vec::new();
    let error;
    {
        let mut writer = FrameWriter::new(&mut output, limits());
        error = writer
            .write_payload(&SerializationProbe::OrdinaryCustomError)
            .unwrap_err();
        assert_eq!(writer.bytes_written(), 0);
    }
    assert!(output.is_empty());
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "frame byte limit");
    assert!(
        !error
            .get_ref()
            .is_some_and(|cause| cause.is::<ProtocolBudgetError>())
    );
}
