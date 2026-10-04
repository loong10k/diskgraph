//! 深树成功与拒绝路径的真实小线程栈销毁；来源：pinned Node 原递归 children 所有权。
use diskgraph_disktree_core::tree::Node;
use diskgraph_scan_worker::{FlatNode, Frame, ProtocolLimits, read_tree, write_frame};

fn stream(depth: u64, end_count: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    for index in 0..depth {
        let mut node = FlatNode::from_native(
            &Node::directory(if index == 0 { "/root" } else { "d" }),
            index,
            index.checked_sub(1),
            index,
        );
        node.child_count = u64::from(index + 1 < depth);
        write_frame(&mut bytes, &Frame::Node { node }).unwrap();
    }
    write_frame(&mut bytes, &Frame::End { nodes: end_count }).unwrap();
    bytes
}

#[test]
fn deep_tree_success_and_failure_dispose_without_recursive_stack_drop() {
    let depth = 10_000;
    let valid = stream(depth, depth);
    let malformed = stream(depth, depth - 1);
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let limits = ProtocolLimits {
                max_frame_bytes: 4096,
                max_stream_bytes: 16 * 1024 * 1024,
                max_nodes: depth,
                max_depth: depth,
            };
            let tree = read_tree(valid.as_slice(), limits).unwrap();
            assert_eq!(tree.name.as_ref(), "/root");
            drop(tree);
            assert!(read_tree(malformed.as_slice(), limits).is_err());
        })
        .unwrap()
        .join()
        .unwrap();
}
