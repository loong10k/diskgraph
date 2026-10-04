//! D35 / Q-04 公开纯值对象矩阵；来源：DiskGraph 原生 Rust 历史查询契约。
//! provider/volume 均为合成身份，不声称执行真实移动端 provider 或挂载切换。

use diskgraph_core::{
    Change, DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, FileIdentity,
    Incompatibility, NodeKind, ResourceLocator, ScanCoverage, ScanSettings,
};

fn graph(time: u64, bytes: u64) -> DiskGraph {
    let root = DiskNode {
        id: 1,
        parent_id: None,
        locator: ResourceLocator::NativePath("/synthetic".into()),
        name: "synthetic".into(),
        kind: NodeKind::Directory,
        subtree_bytes: bytes,
        direct_bytes: 0,
        size_known: true,
        files: 1,
        directories: 1,
        modified_unix_seconds: Some(100),
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    };
    let file = DiskNode {
        id: 2,
        parent_id: Some(1),
        locator: ResourceLocator::NativePath("/synthetic/item".into()),
        name: "item".into(),
        kind: NodeKind::File,
        direct_bytes: bytes,
        directories: 0,
        file_identity: Some(FileIdentity {
            volume_id: "synthetic-volume".into(),
            file_id: 42,
        }),
        ..root.clone()
    };
    DiskGraph {
        snapshot: DiskSnapshot {
            id: format!("observation-{time}"),
            root: root.locator.clone(),
            volume_id: Some("synthetic-volume".into()),
            captured_at_unix_ms: time,
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
        nodes: vec![root, file],
        evidence: Vec::new(),
    }
}

fn reject(before: &DiskGraph, after: &DiskGraph, reason: Incompatibility) {
    assert!(after.growth(before, &before.nodes[1].locator).is_none());
    let result = after.changes(before);
    assert_eq!(result.incompatible, Some(reason));
    assert!(result.changes.is_empty());
}

#[test]
fn all_scan_settings_are_part_of_history_compatibility() {
    let variants: [fn(&mut ScanSettings); 6] = [
        |s| s.apparent_size = false,
        |s| s.follow_links = true,
        |s| s.include_hidden = false,
        |s| s.one_filesystem = false,
        |s| s.max_depth = Some(1),
        |s| s.dedup_hardlinks = true,
    ];
    for change in variants {
        for left in [false, true] {
            let mut before = graph(1, 100);
            let mut after = graph(2, 200);
            change(if left {
                &mut before.snapshot.settings
            } else {
                &mut after.snapshot.settings
            });
            reject(&before, &after, Incompatibility::DifferentSettings);
        }
    }
}

#[test]
fn volume_presence_value_order_and_partial_coverage_are_explicit() {
    for left in [false, true] {
        for (volume, reason) in [
            (None, Incompatibility::UnknownVolume),
            (Some("changed-volume"), Incompatibility::DifferentVolume),
            (Some("provider:other"), Incompatibility::DifferentVolume),
        ] {
            let mut before = graph(1, 100);
            let mut after = graph(2, 200);
            let target = if left { &mut before } else { &mut after };
            target.snapshot.volume_id = volume.map(str::to_owned);
            reject(&before, &after, reason);
        }
        for mode in 0..3 {
            let mut before = graph(1, 100);
            let mut after = graph(2, 200);
            let target = if left { &mut before } else { &mut after };
            target.snapshot.coverage = ScanCoverage {
                complete: false,
                unreadable_nodes: u64::from(mode == 1),
                depth_limited: mode == 2,
            };
            reject(&before, &after, Incompatibility::IncompleteCoverage);
        }
    }
    reject(&graph(2, 100), &graph(1, 200), Incompatibility::OutOfOrder);
}

#[test]
fn contradictory_complete_coverage_never_yields_known_growth() {
    for left in [false, true] {
        for unreadable in [false, true] {
            let mut before = graph(1, 100);
            let mut after = graph(2, 200);
            let target = if left { &mut before } else { &mut after };
            target.snapshot.coverage.unreadable_nodes = u64::from(unreadable);
            target.snapshot.coverage.depth_limited = !unreadable;
            assert!(after.growth(&before, &before.nodes[1].locator).is_none());
        }
    }
}

#[test]
fn contradictory_complete_coverage_never_claims_removed_or_resized_nodes() {
    for left in [false, true] {
        for unreadable in [false, true] {
            let mut before = graph(1, 100);
            let mut after = graph(2, 200);
            after.nodes.truncate(1);
            let target = if left { &mut before } else { &mut after };
            target.snapshot.coverage.unreadable_nodes = u64::from(unreadable);
            target.snapshot.coverage.depth_limited = !unreadable;
            let changes = after.changes(&before);
            assert_eq!(
                changes.incompatible,
                Some(Incompatibility::IncompleteCoverage)
            );
            assert!(changes.changes.is_empty());
        }
    }
}

#[test]
fn contradictory_complete_coverage_never_promotes_rebuildable_candidate() {
    let mut complete = graph(1, 100);
    complete.evidence.push(EvidenceEdge {
        node_id: 1,
        relation: EvidenceRelation::Rebuildable,
        subject: "synthetic-cache".into(),
        source: "D35-public-value-fixture".into(),
        observed_at_unix_ms: 1,
        confidence: 100,
    });
    assert_eq!(complete.candidates(1).len(), 1);
    for unreadable in [false, true] {
        let mut partial = complete.clone();
        partial.snapshot.coverage.unreadable_nodes = u64::from(unreadable);
        partial.snapshot.coverage.depth_limited = !unreadable;
        assert!(partial.candidates(1).is_empty());
    }
}

#[test]
fn unchanged_locator_allows_zero_negative_and_positive_growth_without_id_equality() {
    let before = graph(1, 100);
    for (bytes, delta) in [(0, -100), (100, 0), (200, 100)] {
        let mut after = graph(2, bytes);
        after.nodes[1].file_identity.as_mut().unwrap().file_id = 99;
        assert_eq!(
            after
                .growth(&before, &before.nodes[1].locator)
                .unwrap()
                .delta_bytes,
            delta
        );
        let changes = after.changes(&before);
        let changed_child = changes.changes.iter().any(|change| {
            matches!(change, Change::SizeChanged { node, previous_bytes: 100 } if node.id == 2)
        });
        assert_eq!(changed_child, delta != 0);
    }
}

#[test]
fn same_identity_rename_stays_added_and_removed_without_inferred_growth() {
    let before = graph(1, 100);
    let mut after = graph(2, 100);
    after.nodes[1].name = "renamed".into();
    after.nodes[1].locator = ResourceLocator::NativePath("/synthetic/renamed".into());
    for locator in [&before.nodes[1].locator, &after.nodes[1].locator] {
        assert!(after.growth(&before, locator).is_none());
    }
    let changes = after.changes(&before);
    assert_eq!(changes.changes.len(), 2);
    assert!(
        changes
            .changes
            .iter()
            .any(|c| matches!(c, Change::Added { node } if node.name == "renamed"))
    );
    assert!(
        changes
            .changes
            .iter()
            .any(|c| matches!(c, Change::Removed { name: "item", .. }))
    );
}

#[test]
fn opaque_document_uri_history_uses_exact_locator_and_known_provider_domain() {
    let mut before = graph(1, 100);
    let mut after = graph(2, 200);
    for graph in [&mut before, &mut after] {
        graph.snapshot.root = ResourceLocator::DocumentUri("content://fixture/tree/root".into());
        graph.nodes[0].locator = graph.snapshot.root.clone();
        graph.nodes[1].locator =
            ResourceLocator::DocumentUri("content://fixture/document/item".into());
        graph.snapshot.volume_id = Some("synthetic-provider:account".into());
    }
    assert_eq!(
        after
            .growth(&before, &before.nodes[1].locator)
            .unwrap()
            .delta_bytes,
        100
    );
    assert!(after.changes(&before).incompatible.is_none());
    let mut changed = after.clone();
    changed.snapshot.volume_id = Some("synthetic-provider:other-account".into());
    reject(&before, &changed, Incompatibility::DifferentVolume);
    let mut changed = after;
    changed.snapshot.root = ResourceLocator::NativePath("content://fixture/tree/root".into());
    changed.nodes[0].locator = changed.snapshot.root.clone();
    reject(&before, &changed, Incompatibility::DifferentRoot);
}
