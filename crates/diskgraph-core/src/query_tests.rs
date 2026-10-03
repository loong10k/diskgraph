//! 既有查询行为回归；来源：DiskGraph 原生 Rust query 测试。
use super::{Change, Incompatibility, SizeFilter, TreeRenderError, render_tree};
use crate::{
    DiskGraph, DiskNode, DiskSnapshot, EvidenceEdge, EvidenceRelation, NodeKind, ResourceLocator,
    ScanCoverage, ScanSettings,
};

fn node(id: u64, parent_id: Option<u64>, name: &str, bytes: u64) -> DiskNode {
    DiskNode {
        id,
        parent_id,
        locator: ResourceLocator::NativePath(format!("/tmp/{name}")),
        name: name.to_owned(),
        kind: NodeKind::Directory,
        subtree_bytes: bytes,
        size_known: true,
        direct_bytes: 0,
        files: 0,
        directories: 1,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    }
}

fn graph(bytes: u64) -> DiskGraph {
    DiskGraph {
        snapshot: DiskSnapshot {
            id: "snapshot".into(),
            root: ResourceLocator::NativePath("/tmp".into()),
            volume_id: Some("test-volume".into()),
            captured_at_unix_ms: bytes,
            settings: ScanSettings {
                apparent_size: false,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes: vec![
            node(1, None, "root", bytes),
            node(2, Some(1), "cache", bytes),
            node(3, Some(2), "protected", 10),
        ],
        evidence: vec![EvidenceEdge {
            node_id: 2,
            relation: EvidenceRelation::Rebuildable,
            subject: "build output".into(),
            source: "test".into(),
            observed_at_unix_ms: 0,
            confidence: 80,
        }],
    }
}

#[test]
fn queries_are_bounded_and_growth_requires_complete_compatible_scans() {
    let before = graph(100);
    let after = graph(150);
    assert_eq!(after.top(1, 1)[0].id, 2);
    assert_eq!(after.children(1, 0, 1).items.len(), 1);
    assert_eq!(after.explain(2).unwrap().evidence.len(), 1);
    assert_eq!(
        after
            .growth(&before, &ResourceLocator::NativePath("/tmp/cache".into()))
            .unwrap()
            .delta_bytes,
        50
    );
    let mut partial = before;
    partial.snapshot.coverage.complete = false;
    assert!(after.growth(&partial, &after.nodes[1].locator).is_none());
}

#[test]
fn candidates_never_include_a_protected_descendant() {
    let mut graph = graph(150);
    assert_eq!(graph.candidates(100).len(), 1);
    graph.evidence.push(EvidenceEdge {
        node_id: 3,
        relation: EvidenceRelation::Protected,
        subject: "active data".into(),
        source: "test".into(),
        observed_at_unix_ms: 0,
        confidence: 100,
    });
    assert!(graph.candidates(100).is_empty());
    graph.snapshot.coverage.complete = false;
    graph.evidence.pop();
    assert!(graph.candidates(100).is_empty());
}

#[test]
fn candidates_do_not_treat_unknown_directory_size_as_reclaimable() {
    let mut observed = graph(150);
    observed.nodes[1].size_known = false;
    assert!(observed.candidates(100).is_empty());
    observed.nodes[1].size_known = true;
    observed.nodes[1].read_error = true;
    assert!(observed.candidates(100).is_empty());
}

/// root → cache(large) → {beta, alpha}; cache carries a read error.
fn tree_fixture() -> DiskGraph {
    let mut graph = graph(100);
    graph.nodes.retain(|node| node.id != 3); // drop the protected child
    let mut alpha = node(4, Some(2), "alpha", 10);
    alpha.kind = crate::NodeKind::Directory;
    alpha.directories = 1;
    let mut beta = node(5, Some(2), "beta", 20);
    beta.kind = crate::NodeKind::Directory;
    beta.directories = 1;
    graph.nodes.push(alpha);
    graph.nodes.push(beta);
    graph
}

#[test]
fn tree_view_sorts_children_largest_first() {
    let graph = tree_fixture();
    let view = render_tree(&graph, 3, 0).unwrap();
    let cache = &view.root["children"][0];
    let names: Vec<&str> = cache["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kid| kid["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["beta", "alpha"], "largest first");
    assert_eq!(cache["truncated"], serde_json::Value::Null);
}

#[test]
fn tree_view_marks_truncation_and_hides_below_min_bytes() {
    let graph = tree_fixture();

    // Depth 2 cuts inside cache: the cut is declared, never silent.
    let view = render_tree(&graph, 2, 0).unwrap();
    let cache = &view.root["children"][0];
    assert_eq!(cache["truncated"], true);
    assert_eq!(cache["children_count"], 2);
    assert!(cache.get("children").is_none());

    // A min-bytes floor hides the small child and says how many.
    let view = render_tree(&graph, 3, 15).unwrap();
    let cache = &view.root["children"][0];
    let names: Vec<&str> = cache["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kid| kid["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["beta"]);
    assert_eq!(cache["hidden_below_min_bytes"], 1);
}

#[test]
fn tree_view_carries_read_errors_and_requires_a_root() {
    let mut graph = tree_fixture();
    graph.nodes[1].read_error = true; // cache
    let view = render_tree(&graph, 3, 0).unwrap();
    assert_eq!(view.root["children"][0]["read_error"], true);

    let mut empty = graph;
    empty.nodes.clear();
    assert!(matches!(
        render_tree(&empty, 2, 0),
        Err(TreeRenderError::NoRoot)
    ));
}

#[test]
fn snapshot_json_round_trip_preserves_locators() {
    let graph = graph(100);
    let encoded = serde_json::to_string(&graph).unwrap();
    let decoded: DiskGraph = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, graph);
}

#[test]
fn changes_report_additions_removals_and_size_shifts_without_renames() {
    let before = graph(100);
    let mut after = graph(150);
    let mut extra = node(4, Some(1), "added", 25);
    extra.locator = ResourceLocator::NativePath("/tmp/added".into());
    after.nodes.push(extra);

    let report = after.changes(&before);
    assert!(report.incompatible.is_none());
    assert!(
        report
            .changes
            .iter()
            .any(|change| matches!(change, Change::Added { node } if node.name == "added"))
    );
    assert!(report.changes.iter().any(|change| matches!(
        change,
        Change::SizeChanged {
            previous_bytes: 100,
            ..
        }
    )));

    let mut pruned = graph(150);
    pruned.nodes.retain(|node| node.id != 3);
    let report = pruned.changes(&before);
    assert!(report.changes.iter().any(|change| matches!(
        change,
        Change::Removed { name, .. } if *name == "protected"
    )));
}

#[test]
fn changes_refuse_incompatible_snapshots_with_reasons() {
    let before = graph(100);
    let mut after = graph(100);
    after.snapshot.volume_id = Some("other-volume".into());
    assert!(matches!(
        after.changes(&before).incompatible,
        Some(Incompatibility::DifferentVolume)
    ));
    let mut partial = graph(100);
    partial.snapshot.coverage.complete = false;
    assert!(matches!(
        partial.changes(&before).incompatible,
        Some(Incompatibility::IncompleteCoverage)
    ));
    let mut other_root = graph(100);
    other_root.snapshot.root = ResourceLocator::NativePath("/elsewhere".into());
    other_root.nodes[0].locator = ResourceLocator::NativePath("/elsewhere".into());
    assert!(matches!(
        other_root.changes(&before).incompatible,
        Some(Incompatibility::DifferentRoot)
    ));
}

#[test]
fn children_filtering_keeps_unknown_sizes_visible_and_out_of_byte_order() {
    let mut graph = graph(150);
    // A node whose size could not be reported.
    let mut unknown = node(5, Some(1), "denied", 0);
    unknown.size_known = false;
    unknown.read_error = true;
    graph.nodes.push(unknown);

    // Default listing sorts by bytes and counts the unknown separately.
    // The root owns `cache` and the unsized `denied`; `protected` sits
    // under `cache`.
    let listing = graph.children_filtered(1, None, 0, 10);
    assert_eq!(listing.items.len(), 1);
    assert_eq!(listing.items[0].name, "cache");
    assert_eq!(listing.unknown_count, 1);
    assert!(listing.items.iter().all(|node| node.size_known));

    // Unknown-only surfaces exactly the unsized node.
    let unknown_only = graph.children_filtered(1, Some(SizeFilter::UnknownOnly), 0, 10);
    assert_eq!(unknown_only.items.len(), 1);
    assert_eq!(unknown_only.items[0].name, "denied");

    // A size threshold never admits an unknown size as zero.
    let threshold = graph.children_filtered(1, Some(SizeFilter::AtLeast(100)), 0, 10);
    assert!(threshold.items.iter().all(|node| node.subtree_bytes >= 100));
    assert_eq!(threshold.unknown_count, 1);

    // Paging over the two direct children is stable and resumable.
    let unsized_first = graph.children_filtered(1, Some(SizeFilter::UnknownOnly), 0, 1);
    let unsized_all = graph.children_filtered(1, Some(SizeFilter::UnknownOnly), 0, 10);
    assert_eq!(unsized_first.items.len(), 1);
    assert_eq!(unsized_all.next_offset, None);
    // A zero threshold admits every node with a known size.
    let all_known = graph.children_filtered(1, Some(SizeFilter::AtLeast(0)), 0, 10);
    assert_eq!(all_known.items.len(), 1);
    assert_eq!(all_known.unknown_count, 1);
}
