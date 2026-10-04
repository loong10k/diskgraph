//! 当前 OS 的真实原生路径与 child component 区分；不把 root 展示名当授权路径。
use diskgraph_disktree_core::tree::{Node, NodeKind};
use diskgraph_scan_worker::{FlatNode, Frame, NativePath, ProtocolLimits, read_tree, write_frame};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 4096,
        max_stream_bytes: 32768,
        max_nodes: 10,
        max_depth: 10,
    }
}

#[test]
fn raw_native_absolute_root_roundtrips_without_lossy_conversion() {
    #[cfg(unix)]
    let native = NativePath::Unix(vec![b'/', 0xff, b'x']);
    #[cfg(windows)]
    let native = NativePath::Windows(vec![67, 58, 92, 0xd800, 120]);
    let path = native.to_path().unwrap();
    assert_eq!(NativePath::from_path(&path), native);
}

#[test]
fn a_completed_branch_cannot_be_reopened_later_in_preorder() {
    let mut root = Node::directory("/root");
    root.children.push(Node::directory("first"));
    root.children.push(Node::directory("second"));
    let mut first = FlatNode::from_native(&root.children[0], 1, Some(0), 1);
    first.child_count = 1;
    let leaf = Node::entry("leaf", NodeKind::File, 1);
    let records = [
        FlatNode::from_native(&root, 0, None, 0),
        first,
        FlatNode::from_native(&root.children[1], 2, Some(0), 1),
        FlatNode::from_native(&leaf, 3, Some(1), 2),
    ];
    let mut bytes = Vec::new();
    for node in records {
        write_frame(&mut bytes, &Frame::Node { node }).unwrap();
    }
    write_frame(&mut bytes, &Frame::End { nodes: 4 }).unwrap();
    assert!(read_tree(bytes.as_slice(), limits()).is_err());
}

#[cfg(unix)]
#[test]
fn unix_backslash_and_colon_are_valid_single_components() {
    let mut root = Node::directory("/root");
    root.children.push(Node::entry("a\\b:c", NodeKind::File, 7));
    let mut bytes = Vec::new();
    diskgraph_scan_worker::write_tree_with_limits(&mut bytes, &root, limits()).unwrap();
    let tree = read_tree(bytes.as_slice(), limits()).unwrap();
    assert_eq!(tree.children[0].name.as_ref(), "a\\b:c");
}
