//! 原生 Rust pinned Node/ScanOptions/ScanSnapshot 配对；不调用 OS helper，不声称退出或授权。
use diskgraph_disktree_core::{
    classify::{Category, Reclaim},
    scan::{ScanOptions as NativeOptions, ScanSnapshot},
    tree::{Metric, Node, NodeKind},
};
use diskgraph_scan_worker::{
    FlatNode, FlatNodes, Frame, FrameReader, NativePath, ProtocolLimits, ScanOptions, ScanProgress,
    ScanRequest, read_tree, write_frame, write_tree,
};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 16_384,
        max_stream_bytes: 4_194_304,
        max_nodes: 1024,
        max_depth: 512,
    }
}

#[test]
fn all_native_node_fields_and_sibling_order_round_trip() {
    let categories = [
        Category::Code,
        Category::AgentScratch,
        Category::Toolchain,
        Category::Synced,
        Category::Git,
        Category::Media,
        Category::Documents,
        Category::Cache,
        Category::Other,
    ];
    let reclaims = [
        Reclaim::Regenerable,
        Reclaim::SyncHistory,
        Reclaim::PackageStore,
        Reclaim::BuildOutput,
        Reclaim::Reinstallable,
        Reclaim::SandboxLayers,
        Reclaim::Snapshots,
        Reclaim::Trash,
        Reclaim::Temporary,
    ];
    let kinds = [
        NodeKind::Directory,
        NodeKind::File,
        NodeKind::Symlink,
        NodeKind::Other,
    ];
    let mut root = Node::directory("/real-root");
    for (i, (category, reclaim)) in categories.into_iter().zip(reclaims).enumerate() {
        let mut node = Node::entry(format!("entry-{i}"), kinds[i % kinds.len()], 900 + i as u64);
        node.own_bytes = 17 + i as u64;
        node.files = 123;
        node.own_files = 4;
        node.dirs = 5;
        node.inode = Some((u64::MAX - i as u64, u64::MAX));
        node.read_error = i % 2 == 0;
        node.modified = -123 + i as i64;
        node.category = category;
        node.reclaim = Some(reclaim);
        if node.kind == NodeKind::Directory {
            node.children
                .push(Node::entry("nested", NodeKind::File, 77));
        }
        root.children.push(node);
    }
    let expected: Vec<_> = FlatNodes::new(&root).collect::<Result<_, _>>().unwrap();
    let mut bytes = Vec::new();
    assert_eq!(
        write_tree(&mut bytes, &root).unwrap(),
        expected.len() as u64
    );
    let decoded = read_tree(bytes.as_slice(), limits()).unwrap();
    let actual: Vec<_> = FlatNodes::new(&decoded).collect::<Result<_, _>>().unwrap();
    assert_eq!(actual, expected);
    assert_eq!(decoded.children[0].name.as_ref(), "entry-0");
    assert_eq!(decoded.children[8].name.as_ref(), "entry-8");
}

#[test]
fn options_preserve_each_boolean_depth_and_metric() {
    for mask in 0..32 {
        for metric in [Metric::Bytes, Metric::Files] {
            for max_depth in [None, Some(0), Some(300)] {
                let native = NativeOptions {
                    apparent_size: mask & 1 != 0,
                    follow_links: mask & 2 != 0,
                    include_hidden: mask & 4 != 0,
                    one_filesystem: mask & 8 != 0,
                    dedup_hardlinks: mask & 16 != 0,
                    max_depth,
                    metric,
                };
                let wire = ScanOptions::from_native(&native);
                assert_eq!(ScanOptions::from_native(&wire.to_native().unwrap()), wire);
            }
        }
    }
}

#[test]
fn progress_preserves_error_messages_and_finished_is_only_a_field() {
    let snapshot = ScanSnapshot {
        files: u64::MAX,
        dirs: 17,
        bytes: 39,
        errors: 5,
        finished: true,
        cancelled: true,
        messages: vec!["actual unreadable detail".into(), "second".into()],
    };
    let frame = Frame::Progress {
        progress: ScanProgress::from_native(snapshot.clone()),
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &frame).unwrap();
    let mut reader = FrameReader::new(bytes.as_slice(), limits());
    let Some(Frame::Progress { progress }) = reader.read_frame().unwrap() else {
        panic!("progress expected")
    };
    assert_eq!(progress.into_native(), snapshot);
    assert!(reader.read_frame().unwrap().is_none());
    assert_eq!(reader.bytes_read(), bytes.len() as u64);
}

#[test]
fn requests_preserve_unix_bytes_and_windows_unpaired_utf16_on_the_wire() {
    for root in [
        NativePath::Unix(vec![b'/', 0xff, b'x']),
        NativePath::Windows(vec![b'C' as u16, 58, 92, 0xd800, b'x' as u16]),
    ] {
        let request = ScanRequest {
            root,
            options: ScanOptions::from_native(&NativeOptions::default()),
        };
        let frame = Frame::Request { request };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).unwrap();
        assert_eq!(
            FrameReader::new(bytes.as_slice(), limits())
                .read_frame()
                .unwrap(),
            Some(frame)
        );
    }
}

#[test]
fn closed_native_variant_tags_are_not_defaulted() {
    let base = FlatNode::from_native(&Node::directory("/root"), 0, None, 0);
    for field in 0..3 {
        let mut node = base.clone();
        match field {
            0 => node.kind = 255,
            1 => node.category = 255,
            _ => node.reclaim = Some(255),
        }
        assert!(node.into_native().is_err());
    }
    let mut options = ScanOptions::from_native(&NativeOptions::default());
    options.metric = 255;
    assert!(options.to_native().is_err());
}

#[test]
fn deep_flat_sequence_uses_original_parent_and_depth() {
    let mut root = Node::entry("leaf", NodeKind::File, 1);
    for _ in 0..300 {
        let mut parent = Node::directory("dir");
        parent.children.push(root);
        root = parent;
    }
    let flat: Vec<_> = FlatNodes::new(&root).collect::<Result<_, _>>().unwrap();
    assert_eq!(flat.len(), 301);
    for (i, node) in flat.iter().enumerate() {
        assert_eq!(node.sequence, i as u64);
        assert_eq!(node.depth, i as u64);
        assert_eq!(node.parent, i.checked_sub(1).map(|n| n as u64));
    }
    let mut bytes = Vec::new();
    write_tree(&mut bytes, &root).unwrap();
    assert_eq!(
        FlatNodes::new(&read_tree(bytes.as_slice(), limits()).unwrap()).count(),
        301
    );
}
