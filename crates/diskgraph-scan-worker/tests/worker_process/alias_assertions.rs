//! 真实别名的专用跨运行验收；来源：pinned Seen 并行先到计费，非全局字段忽略器。
use super::worker_output::WorkerOutput;
use diskgraph_disktree_core::{
    scan::ScanSnapshot,
    tree::{Metric, Node},
};
use diskgraph_scan_worker::{
    DecodedTree, FlatNode, FlatNodes, Frame, ProtocolLimits, read_tree, write_frame,
    write_tree_with_limits,
};
use std::collections::BTreeMap;

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 1_048_576,
        max_stream_bytes: 8_388_608,
        max_nodes: 10_000,
        max_depth: 128,
    }
}

fn flat(root: &Node) -> Vec<FlatNode> {
    FlatNodes::new(root).collect::<Result<_, _>>().unwrap()
}

fn same_run_exact(root: &Node) {
    let expected = flat(root);
    let mut wire = Vec::new();
    write_tree_with_limits(&mut wire, root, limits()).unwrap();
    let decoded = read_tree(wire.as_slice(), limits()).unwrap();
    assert_eq!(
        flat(&decoded),
        expected,
        "same-run all fields and order remain exact"
    );
}

fn decode_actual(output: &WorkerOutput) -> DecodedTree {
    assert!(
        output.status.success(),
        "actual helper exit {:?}",
        output.status
    );
    let mut wire = Vec::new();
    for value in &output.frames {
        if matches!(value["type"].as_str(), Some("node" | "end")) {
            let frame: Frame = serde_json::from_value(value.clone()).unwrap();
            write_frame(&mut wire, &frame).unwrap();
        }
    }
    // 原平铺 parent/sequence/count 必须先完整合法，不能先按名字排序抹去错误结构。
    read_tree(wire.as_slice(), limits()).unwrap()
}

fn by_path(nodes: &[FlatNode]) -> BTreeMap<Vec<String>, &FlatNode> {
    let mut paths: Vec<Vec<String>> = Vec::new();
    let mut indexed = BTreeMap::new();
    for node in nodes {
        assert_eq!(node.sequence as usize, paths.len());
        let mut path = node
            .parent
            .map_or_else(Vec::new, |parent| paths[parent as usize].clone());
        path.push(node.name.clone());
        assert!(
            indexed.insert(path.clone(), node).is_none(),
            "duplicate full path"
        );
        paths.push(path);
    }
    indexed
}

fn assert_sorted(nodes: &[FlatNode], metric: Metric) {
    let mut siblings = BTreeMap::<Option<u64>, Vec<&FlatNode>>::new();
    for node in nodes {
        siblings.entry(node.parent).or_default().push(node);
    }
    for children in siblings.values() {
        for pair in children.windows(2) {
            let value = |node: &FlatNode| match metric {
                Metric::Bytes => node.bytes,
                Metric::Files => node.files,
            };
            let left = value(pair[0]);
            let right = value(pair[1]);
            assert!(
                left > right || (left == right && pair[0].name <= pair[1].name),
                "each run retains pinned metric-desc/name-asc sibling order: {pair:?}"
            );
        }
    }
}

fn assert_single_charge(nodes: &[FlatNode], identity: (u64, u64), single_size: u64) {
    let group: Vec<_> = nodes
        .iter()
        .filter(|node| node.inode == Some(identity))
        .collect();
    assert!(
        group.len() >= 2,
        "actual hardlink identity must be captured, never skipped"
    );
    assert_eq!(group.iter().filter(|node| node.bytes > 0).count(), 1);
    assert_eq!(
        group.iter().map(|node| node.bytes).sum::<u64>(),
        single_size,
        "actual apparent file size is charged exactly once"
    );
    for node in group {
        assert_eq!(node.bytes, node.own_bytes);
        assert!(node.bytes == 0 || node.bytes == single_size);
    }
}

/// 参数：output 是实际 helper 结果，reference 是独立 pinned 结果，single_size 为真实元数据文件长度。
/// 返回：无返回值；只对已资格共享 inode 允许计费名称变化，其余全字段、路径与每次自身顺序必须成立。
pub(crate) fn assert_dedup_success(
    output: &WorkerOutput,
    reference: &Node,
    progress: &ScanSnapshot,
    metric: Metric,
    single_size: u64,
) {
    let actual = decode_actual(output);
    crate::assert_success(output, &actual, progress);
    same_run_exact(reference);
    same_run_exact(&actual);
    let expected = flat(reference);
    let actual = flat(&actual);
    let identity = expected
        .iter()
        .find(|node| node.name == "ordinary.txt")
        .unwrap()
        .inode
        .expect("real shared native identity required; unsupported is not a skipped success");
    assert_single_charge(&expected, identity, single_size);
    assert_single_charge(&actual, identity, single_size);
    assert_sorted(&expected, metric);
    assert_sorted(&actual, metric);
    let expected_paths = by_path(&expected);
    let actual_paths = by_path(&actual);
    assert_eq!(
        actual_paths.keys().collect::<Vec<_>>(),
        expected_paths.keys().collect::<Vec<_>>()
    );
    for (path, actual_node) in &actual_paths {
        let original = expected_paths[path];
        let mut expected_node = original.clone();
        // 两棵树已通过原序号/父结构验证且全路径相等；不同 sibling 排序可以改变这些派生索引。
        expected_node.sequence = actual_node.sequence;
        expected_node.parent = actual_node.parent;
        if original.inode == Some(identity) {
            assert_eq!(actual_node.inode, Some(identity));
            expected_node.bytes = actual_node.bytes;
            expected_node.own_bytes = actual_node.own_bytes;
        }
        assert_eq!(
            &expected_node, *actual_node,
            "all other fields, including non-alias bytes and parent path, remain exact: {path:?}"
        );
    }
    assert_eq!(
        actual[0], expected[0],
        "root counts/bytes/all fields stay exact"
    );
}
