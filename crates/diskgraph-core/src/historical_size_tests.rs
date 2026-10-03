//! D33 / Q-04 公共查询回归：节点未知事实不得产生确定尺寸结论。

use crate::{
    Change, DifferentReason, DiskGraph, DiskNode, DiskSnapshot, Evidence, FileIdentity, NodeKind,
    ResourceLocator, ScanCoverage, ScanSettings, Verdict,
};

fn node(path: &str, kind: NodeKind, bytes: u64) -> DiskNode {
    DiskNode {
        id: 2,
        parent_id: Some(1),
        locator: ResourceLocator::NativePath(path.into()),
        name: "entry".into(),
        kind,
        subtree_bytes: bytes,
        direct_bytes: bytes,
        size_known: true,
        files: 1,
        directories: u64::from(kind == NodeKind::Directory),
        modified_unix_seconds: Some(500),
        file_identity: Some(FileIdentity {
            volume_id: "volume".into(),
            file_id: 7,
        }),
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    }
}

fn graph(kind: NodeKind, bytes: u64) -> DiskGraph {
    let root = ResourceLocator::NativePath("/same".into());
    let mut root_node = node("/same", NodeKind::Directory, bytes);
    root_node.id = 1;
    root_node.parent_id = None;
    DiskGraph {
        snapshot: DiskSnapshot {
            id: "snapshot".into(),
            root,
            volume_id: Some("volume".into()),
            captured_at_unix_ms: 100,
            settings: ScanSettings {
                apparent_size: true,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: false,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![root_node, node("/same/entry", kind, bytes)],
        evidence: Vec::new(),
    }
}

#[test]
fn growth_refuses_each_unknown_or_read_error_side_independently() {
    for left_side in [true, false] {
        for read_error in [true, false] {
            let mut before = graph(NodeKind::File, 100);
            let mut after = graph(NodeKind::File, 150);
            let target = if left_side {
                &mut before.nodes[1]
            } else {
                &mut after.nodes[1]
            };
            if read_error {
                target.read_error = true;
            } else {
                target.size_known = false;
            }
            assert!(
                after.growth(&before, &after.nodes[1].locator).is_none(),
                "left_side={left_side}, read_error={read_error}"
            );
        }
    }
}

#[test]
fn changes_never_count_unobserved_bytes_as_size_changed() {
    for left_side in [true, false] {
        for read_error in [true, false] {
            let mut before = graph(NodeKind::Directory, 100);
            let mut after = graph(NodeKind::Directory, 150);
            let target = if left_side {
                &mut before.nodes[1]
            } else {
                &mut after.nodes[1]
            };
            if read_error {
                target.read_error = true;
            } else {
                target.size_known = false;
            }
            let changes = after.changes(&before);
            assert!(
                !changes.changes.iter().any(|change| matches!(change,
                Change::SizeChanged { node, .. } if node.id == 2)),
                "left_side={left_side}, read_error={read_error}"
            );
        }
    }
}

#[test]
fn type_replacement_has_no_growth_or_numeric_size_change() {
    for (before_kind, after_kind) in [
        (NodeKind::File, NodeKind::Directory),
        (NodeKind::Directory, NodeKind::File),
        (NodeKind::File, NodeKind::Symlink),
        (NodeKind::Symlink, NodeKind::File),
        (NodeKind::File, NodeKind::Other),
    ] {
        let before = graph(before_kind, 100);
        let after = graph(after_kind, 150);
        assert!(
            after.growth(&before, &after.nodes[1].locator).is_none(),
            "{before_kind:?} -> {after_kind:?}"
        );
        assert!(
            !after
                .changes(&before)
                .changes
                .iter()
                .any(|change| matches!(change,
            Change::SizeChanged { node, .. } if node.id == 2))
        );
    }
}

#[test]
fn each_unknown_or_read_error_side_prevents_same_metadata_for_files_and_directories() {
    for kind in [NodeKind::File, NodeKind::Directory] {
        for left_side in [true, false] {
            for read_error in [true, false] {
                let mut left = node("/a/entry", kind, 0);
                let mut right = node("/b/entry", kind, 0);
                let target = if left_side { &mut left } else { &mut right };
                if read_error {
                    target.read_error = true;
                } else {
                    target.size_known = false;
                }
                assert_eq!(
                    crate::compare::compare_entry(&left, &right, 0),
                    Verdict::Different {
                        reason: DifferentReason::UnknownSize
                    },
                    "kind={kind:?}, left_side={left_side}, read_error={read_error}"
                );
            }
        }
    }
}

#[test]
fn two_unknown_directories_remain_unknown_in_comparison_and_rollup() {
    let mut left = graph(NodeKind::Directory, 0);
    let mut right = graph(NodeKind::Directory, 0);
    left.nodes[1].size_known = false;
    right.nodes[1].size_known = false;
    let (rows, summary) = crate::compare(
        &left.nodes[0],
        &left.nodes,
        &right.nodes[0],
        &right.nodes,
        0,
    );
    assert_eq!(summary.same, 0);
    assert_eq!(summary.unknown, 1);
    assert_eq!(
        crate::compare::roll_up(&[rows[0].verdict.clone()]),
        Verdict::Different {
            reason: DifferentReason::UnknownSize
        }
    );
}

#[test]
fn known_type_replacements_are_path_differences_even_with_equal_metadata() {
    for (left_kind, right_kind) in [
        (NodeKind::File, NodeKind::Directory),
        (NodeKind::Directory, NodeKind::File),
        (NodeKind::File, NodeKind::Symlink),
        (NodeKind::Symlink, NodeKind::File),
        (NodeKind::File, NodeKind::Other),
    ] {
        assert_eq!(
            crate::compare::compare_entry(
                &node("/a/entry", left_kind, 10),
                &node("/b/entry", right_kind, 10),
                0
            ),
            Verdict::Different {
                reason: DifferentReason::Path
            }
        );
    }
}

#[test]
fn separate_roots_and_distinct_file_identities_allow_metadata_comparison() {
    let mut left_root = node("/source", NodeKind::Directory, 10);
    left_root.id = 1;
    left_root.parent_id = None;
    let mut right_root = left_root.clone();
    right_root.locator = ResourceLocator::NativePath("/copy".into());
    let left = node("/source/entry", NodeKind::File, 10);
    let mut right = node("/copy/entry", NodeKind::File, 10);
    right.file_identity = Some(FileIdentity {
        volume_id: "other-volume".into(),
        file_id: 99,
    });
    let left_nodes = [left];
    let right_nodes = [right];
    let (rows, summary) = crate::compare(&left_root, &left_nodes, &right_root, &right_nodes, 0);
    assert_eq!(summary.same, 1);
    assert_eq!(
        rows[0].verdict,
        Verdict::Same {
            evidence: Evidence::Metadata
        }
    );
}

#[test]
fn known_zero_and_negative_path_growth_do_not_require_equal_file_ids() {
    for (after_bytes, expected) in [(100, 0), (0, -100)] {
        let before = graph(NodeKind::File, 100);
        let mut after = graph(NodeKind::File, after_bytes);
        after.nodes[1].file_identity.as_mut().unwrap().file_id = 88;
        assert_eq!(
            after
                .growth(&before, &after.nodes[1].locator)
                .unwrap()
                .delta_bytes,
            expected
        );
    }
}

#[test]
fn unknown_or_read_error_takes_precedence_over_kind_replacement() {
    for left_side in [true, false] {
        for read_error in [true, false] {
            let mut left = node("/a/entry", NodeKind::File, 10);
            let mut right = node("/b/entry", NodeKind::Directory, 20);
            let target = if left_side { &mut left } else { &mut right };
            if read_error {
                target.read_error = true;
            } else {
                target.size_known = false;
            }
            assert_eq!(
                crate::compare::compare_entry(&left, &right, 0),
                Verdict::Different {
                    reason: DifferentReason::UnknownSize
                },
                "left_side={left_side}, read_error={read_error}"
            );
        }
    }
}

#[test]
fn root_growth_refuses_each_unknown_or_read_error_side() {
    for left_side in [true, false] {
        for read_error in [true, false] {
            let mut before = graph(NodeKind::Directory, 100);
            let mut after = graph(NodeKind::Directory, 150);
            let target = if left_side {
                &mut before.nodes[0]
            } else {
                &mut after.nodes[0]
            };
            if read_error {
                target.read_error = true;
            } else {
                target.size_known = false;
            }
            assert!(
                after.growth(&before, &after.snapshot.root).is_none(),
                "left_side={left_side}, read_error={read_error}"
            );
        }
    }
}

#[test]
fn known_directory_changes_count_byte_changes_but_not_count_only_changes() {
    let before = graph(NodeKind::Directory, 100);
    let grown = graph(NodeKind::Directory, 150);
    assert!(grown.changes(&before).changes.iter().any(|change| matches!(
        change,
        Change::SizeChanged { node, previous_bytes: 100 } if node.id == 2 && node.subtree_bytes == 150
    )));
    let mut counts_only = graph(NodeKind::Directory, 100);
    counts_only.nodes[1].files += 1;
    counts_only.nodes[1].directories += 1;
    assert!(
        !counts_only
            .changes(&before)
            .changes
            .iter()
            .any(|change| matches!(
                change,
                Change::SizeChanged { node, .. } if node.id == 2
            ))
    );
}
