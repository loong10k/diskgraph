//! Scanner behavior pinned against isolated fixtures (P0 task 1.7, specs
//! FS-03 / FS-04 / FS-05). These tests document what the pinned disktree
//! revision actually does; changing the pin must revisit every assertion here.

use diskgraph_disktree_core::scan::ScanOptions;

use diskgraph_core::DiskGraph;
use diskgraph_disktree::scan_native;
use diskgraph_testkit::FixtureTree;
#[cfg(unix)]
use diskgraph_testkit::UnreadableDir;

fn find<'a>(graph: &'a DiskGraph, name: &str) -> &'a diskgraph_core::DiskNode {
    graph
        .nodes
        .iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("fixture node {name} missing from scan"))
}

#[cfg(unix)]
#[test]
fn sparse_file_reported_apparent_or_allocated_by_setting() {
    let tree = FixtureTree::new("sparse").unwrap();
    tree.sparse_file("hole.bin", 1 << 20).unwrap();
    let apparent = scan_native(
        tree.path(),
        ScanOptions {
            apparent_size: true,
            ..ScanOptions::default()
        },
    )
    .unwrap();
    let allocated = scan_native(
        tree.path(),
        ScanOptions {
            apparent_size: false,
            ..ScanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(find(&apparent, "hole.bin").subtree_bytes, 1 << 20);
    // APFS/ext4 both keep a fresh set_len file sparse; if a filesystem ever
    // materializes blocks this assertion catches the platform drift.
    assert!(
        find(&allocated, "hole.bin").subtree_bytes <= (1 << 20) / 2,
        "allocated {} must be far below apparent {}",
        find(&allocated, "hole.bin").subtree_bytes,
        1 << 20
    );
}

#[cfg(unix)]
#[test]
fn hardlinks_count_once_with_dedup_and_twice_without() {
    let tree = FixtureTree::new("hardlink").unwrap();
    tree.file("a.bin", 4096).unwrap();
    tree.hardlink("a.bin", "b.bin").unwrap();
    let dedup = scan_native(
        tree.path(),
        ScanOptions {
            apparent_size: true,
            dedup_hardlinks: true,
            ..ScanOptions::default()
        },
    )
    .unwrap();
    let counting = scan_native(
        tree.path(),
        ScanOptions {
            apparent_size: true,
            dedup_hardlinks: false,
            ..ScanOptions::default()
        },
    )
    .unwrap();
    let dedup_root = dedup
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .unwrap();
    let counting_root = counting
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .unwrap();
    assert_eq!(
        counting_root.subtree_bytes,
        dedup_root.subtree_bytes + 4096,
        "the second link must add its bytes again when dedup is off"
    );
}

#[cfg(unix)]
#[test]
fn symlink_loop_neither_hangs_nor_expands_by_default() {
    let tree = FixtureTree::new("loop").unwrap();
    tree.file("real.bin", 16).unwrap();
    tree.symlink(tree.path(), "loop").unwrap();
    let graph = scan_native(tree.path(), ScanOptions::default()).unwrap();
    let symlink = find(&graph, "loop");
    assert_eq!(symlink.kind, diskgraph_core::NodeKind::Symlink);
    assert!(
        graph.snapshot.coverage.complete,
        "a non-followed loop must stay complete"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_directory_forces_incomplete_coverage() {
    let tree = FixtureTree::new("denied").unwrap();
    let locked = tree.dir("locked").unwrap();
    tree.file("locked/private.bin", 32).unwrap();
    let guard = UnreadableDir::make(locked).unwrap();
    let graph = scan_native(tree.path(), ScanOptions::default()).unwrap();
    drop(guard);
    assert!(!graph.snapshot.coverage.complete);
    assert!(graph.snapshot.coverage.unreadable_nodes >= 1);
    // The denied directory is still listed; what it hides is unknown, not empty.
    let denied = find(&graph, "locked");
    assert!(denied.read_error);
}

#[test]
fn hidden_files_are_indexed_but_never_become_candidates() {
    let tree = FixtureTree::new("hidden").unwrap();
    tree.hidden_file("cache", 256).unwrap();
    let graph = scan_native(tree.path(), ScanOptions::default()).unwrap();
    assert!(
        graph.nodes.iter().any(|node| node.name == ".cache"),
        "include_hidden must keep dotfiles observable"
    );
    assert!(graph.candidates(1).is_empty());
    assert!(graph.evidence.is_empty());
}

#[test]
fn depth_limited_scan_keeps_bounded_nodes_but_marks_coverage_partial() {
    let tree = FixtureTree::new("depth").unwrap();
    tree.file("a/b/c/deep.bin", 8).unwrap();
    let graph = scan_native(
        tree.path(),
        ScanOptions {
            max_depth: Some(1),
            ..ScanOptions::default()
        },
    )
    .unwrap();
    assert!(!graph.snapshot.coverage.complete);
    assert!(graph.snapshot.coverage.depth_limited);
    assert!(
        graph.nodes.iter().all(|node| node.name != "deep.bin"),
        "depth cap must stop descent, not silently flatten"
    );
}
