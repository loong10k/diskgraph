//! v2 测试输入只编码明确载荷；来源：现有 FrameWriter 与 pinned Node 全字段。
use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{ExecutionDecoder, ExecutionEvent, FlatNode, Frame, ProtocolLimits};

pub const TARGET: &str = "qualified-native-target";
pub const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

pub fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 131_072,
        max_nodes: 32,
        max_depth: 16,
    }
}

pub fn decoder(bound: ProtocolLimits) -> ExecutionDecoder {
    ExecutionDecoder::new(bound, TARGET, PIN).unwrap()
}

pub fn packet(payload: &impl serde::Serialize) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap();
    let mut bytes = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    bytes.extend(body);
    bytes
}

pub fn hello() -> Vec<u8> {
    packet(&Frame::Hello {
        version: 2,
        target: TARGET.into(),
        pin: PIN.into(),
    })
}

pub fn root(child_count: u64) -> FlatNode {
    let mut node = FlatNode::from_native(&Node::directory("/observed-root"), 0, None, 0);
    node.child_count = child_count;
    node
}

pub fn leaf(sequence: u64, name: &str) -> FlatNode {
    FlatNode::from_native(&Node::entry(name, NodeKind::File, 13), sequence, Some(0), 1)
}

pub fn push_all(decoder: &mut ExecutionDecoder, mut bytes: &[u8]) -> Vec<ExecutionEvent> {
    let mut events = Vec::new();
    while !bytes.is_empty() {
        let (consumed, event) = decoder.push(bytes).unwrap();
        assert!(consumed > 0 && consumed <= bytes.len());
        bytes = &bytes[consumed..];
        if let Some(event) = event {
            events.push(event);
        }
    }
    events
}

pub fn one_node_stream() -> Vec<u8> {
    let mut stream = hello();
    stream.extend(packet(&Frame::Node { node: root(0) }));
    stream.extend(packet(&Frame::End { nodes: 1 }));
    stream
}
