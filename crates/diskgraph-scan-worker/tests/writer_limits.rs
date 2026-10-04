//! 有限写 sink 与阶段门禁：真实 serde/Write，不以最后 body 大小事后拒绝充准入。
use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{
    FlatNode, Frame, FrameWriter, ProtocolLimits, TreeWriter, read_tree, write_frame,
    write_tree_with_limits,
};
use std::io::{self, Write};

/// 观察真实下游 Write 是否收到超额帧；来源：std::io writer 边界测试。
#[derive(Default)]
struct ObservedOutput {
    writes: usize,
    bytes: Vec<u8>,
}
impl Write for ObservedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 32768,
        max_nodes: 10,
        max_depth: 5,
    }
}

fn root() -> Frame {
    Frame::Node {
        node: FlatNode::from_native(&Node::directory("/root"), 0, None, 0),
    }
}

#[test]
fn oversized_serialization_never_writes_any_header_or_body() {
    let mut output = ObservedOutput::default();
    let mut bound = limits();
    bound.max_frame_bytes = 32;
    {
        let mut writer = FrameWriter::new(&mut output, bound);
        let frame = Frame::Error {
            code: "failure".into(),
            message: "x".repeat(2 * 1024 * 1024),
        };
        assert_eq!(
            writer.write_frame(&frame).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(
            writer.write_frame(&Frame::Cancel).is_err(),
            "first error must stay terminal"
        );
        assert_eq!(writer.bytes_written(), 0);
    }
    assert_eq!(output.writes, 0);
    assert!(output.bytes.is_empty());
}

#[test]
fn writer_and_reader_share_exact_header_inclusive_byte_limit() {
    let mut expected = Vec::new();
    write_frame(&mut expected, &Frame::Cancel).unwrap();
    let mut bound = limits();
    bound.max_stream_bytes = expected.len() as u64 * 2 - 1;
    let mut output = Vec::new();
    {
        let mut writer = FrameWriter::new(&mut output, bound);
        assert_eq!(
            writer.write_frame(&Frame::Cancel).unwrap(),
            expected.len() as u64
        );
        assert!(writer.write_frame(&Frame::Cancel).is_err());
        assert_eq!(writer.bytes_written(), expected.len() as u64);
    }
    assert_eq!(output, expected);
}

#[test]
fn result_writer_requires_end_and_rejects_control_or_extra_frames() {
    for unexpected in [
        Frame::Cancel,
        Frame::Hello {
            version: 1,
            target: "target".into(),
            pin: "pin".into(),
        },
    ] {
        let mut bytes = Vec::new();
        let mut writer = TreeWriter::new(&mut bytes, limits());
        assert!(writer.write_frame(&unexpected).is_err());
        assert!(writer.finish().is_err());
        assert!(bytes.is_empty());
    }
    let mut bytes = Vec::new();
    let mut writer = TreeWriter::new(&mut bytes, limits());
    writer.write_frame(&root()).unwrap();
    assert!(writer.finish().is_err());
    let mut bytes = Vec::new();
    let mut writer = TreeWriter::new(&mut bytes, limits());
    writer.write_frame(&root()).unwrap();
    writer.write_frame(&Frame::End { nodes: 1 }).unwrap();
    assert!(writer.write_frame(&root()).is_err());
}

#[test]
fn bounded_writer_rejects_node_and_depth_cost_before_offending_frame() {
    let mut root = Node::directory("/root");
    root.children.push(Node::entry("file", NodeKind::File, 7));
    let mut bound = limits();
    bound.max_nodes = 1;
    let mut bytes = Vec::new();
    assert!(write_tree_with_limits(&mut bytes, &root, bound).is_err());
    assert!(
        bytes.is_empty(),
        "root declaration already exceeds total nodes"
    );
    bound.max_nodes = 2;
    bound.max_depth = 0;
    assert!(write_tree_with_limits(&mut bytes, &root, bound).is_err());
    assert!(
        read_tree(bytes.as_slice(), bound).is_err(),
        "incomplete emitted stream must not become a tree"
    );
}

#[test]
fn result_reader_requires_eof_and_rejects_trailing_even_partial_header() {
    let mut valid = Vec::new();
    write_frame(&mut valid, &root()).unwrap();
    write_frame(&mut valid, &Frame::End { nodes: 1 }).unwrap();
    assert!(read_tree(valid.as_slice(), limits()).is_ok());
    for tail in [vec![0], {
        let mut tail = Vec::new();
        write_frame(&mut tail, &Frame::Cancel).unwrap();
        tail
    }] {
        let mut data = valid.clone();
        data.extend(tail);
        assert_eq!(
            read_tree(data.as_slice(), limits()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn declared_child_count_overflow_is_rejected_without_capacity_allocation() {
    let mut node = FlatNode::from_native(&Node::directory("/root"), 0, None, 0);
    node.child_count = u64::MAX;
    let mut bound = limits();
    bound.max_nodes = u64::MAX;
    let mut bytes = Vec::new();
    let mut writer = TreeWriter::new(&mut bytes, bound);
    assert_eq!(
        writer
            .write_frame(&Frame::Node { node })
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(bytes.is_empty());
}
