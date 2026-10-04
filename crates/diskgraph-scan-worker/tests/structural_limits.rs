//! PF-06 结构边界的真实 runtime RED；使用可解码合法 JSON 帧，不用缺 API/伪 OS 数据。
use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{FlatNode, Frame, NativePath, ProtocolLimits, read_tree, write_frame};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 16_384,
        max_stream_bytes: 1_048_576,
        max_nodes: 20,
        max_depth: 10,
    }
}

fn records() -> Vec<FlatNode> {
    let mut root = Node::directory("/root");
    root.children.push(Node::entry("file", NodeKind::File, 1));
    vec![
        FlatNode::from_native(&root, 0, None, 0),
        FlatNode::from_native(&root.children[0], 1, Some(0), 1),
    ]
}

fn encoded(nodes: Vec<FlatNode>, end: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    for node in nodes {
        write_frame(&mut bytes, &Frame::Node { node }).unwrap();
    }
    write_frame(&mut bytes, &Frame::End { nodes: end }).unwrap();
    bytes
}

#[test]
fn duplicate_sequence_is_refused() {
    let mut nodes = records();
    nodes[1].sequence = 0;
    assert!(
        read_tree(encoded(nodes, 2).as_slice(), limits()).is_err(),
        "duplicate sequence accepted"
    );
}

#[test]
fn root_cannot_have_a_parent() {
    let mut nodes = records();
    nodes[0].parent = Some(0);
    assert!(
        read_tree(encoded(nodes, 2).as_slice(), limits()).is_err(),
        "root parent accepted"
    );
}

#[test]
fn wrong_child_count_and_early_end_are_refused() {
    let mut nodes = records();
    nodes.pop();
    assert!(
        read_tree(encoded(nodes, 1).as_slice(), limits()).is_err(),
        "End while root still needs a child accepted"
    );
}

#[test]
fn end_total_must_match_actual_records() {
    assert!(
        read_tree(encoded(records(), u64::MAX).as_slice(), limits()).is_err(),
        "wrong End count accepted"
    );
}

#[test]
fn child_depth_must_follow_its_actual_parent() {
    let mut nodes = records();
    nodes[1].depth = 0;
    assert!(
        read_tree(encoded(nodes, 2).as_slice(), limits()).is_err(),
        "child depth accepted"
    );
}

#[test]
fn total_nodes_and_depth_use_caller_limits() {
    let data = encoded(records(), 2);
    let mut bound = limits();
    bound.max_nodes = 1;
    assert!(
        read_tree(data.as_slice(), bound).is_err(),
        "node quota ignored"
    );
}

#[test]
fn depth_limit_is_not_a_node_limit() {
    let mut bound = limits();
    bound.max_depth = 0;
    assert!(
        read_tree(encoded(records(), 2).as_slice(), bound).is_err(),
        "depth quota ignored"
    );
}

#[test]
fn current_platform_child_components_cannot_escape_parent() {
    let mut names = vec!["", ".", "..", "/absolute", "a/b", "nul\0byte"];
    if cfg!(windows) {
        names.extend(["a\\b", "C:relative", "C:\\absolute"]);
    }
    for name in names {
        let mut nodes = records();
        nodes[1].name = name.into();
        assert!(
            read_tree(encoded(nodes, 2).as_slice(), limits()).is_err(),
            "illegal child accepted: {name:?}"
        );
    }
}

#[test]
fn root_native_path_must_be_absolute_and_nul_free() {
    #[cfg(unix)]
    let invalid = [
        NativePath::Unix(b"relative".to_vec()),
        NativePath::Unix(b"/x\0y".to_vec()),
    ];
    #[cfg(windows)]
    let invalid = [
        NativePath::Windows("relative".encode_utf16().collect()),
        NativePath::Windows(vec![67, 58, 92, 0]),
    ];
    for root in invalid {
        assert!(root.to_path().is_err(), "illegal root accepted: {root:?}");
    }
}

#[test]
fn forward_parent_is_refused_without_building_a_cycle() {
    let mut nodes = records();
    nodes[1].parent = Some(1);
    assert!(read_tree(encoded(nodes, 2).as_slice(), limits()).is_err());
}

#[test]
fn second_root_is_not_a_child() {
    let mut nodes = records();
    nodes[1].parent = None;
    assert!(read_tree(encoded(nodes, 2).as_slice(), limits()).is_err());
}

#[test]
fn leaf_cannot_own_child_records() {
    let mut nodes = records();
    nodes[0].kind = 1;
    assert!(
        read_tree(encoded(nodes, 2).as_slice(), limits()).is_err(),
        "file with child accepted"
    );
}
