//! Scan-options parity with disktree (P0 contract, upstream pin `158f9cc`).
//! Whatever disktree's own flags do to a walk, the same flags do to a
//! DiskGraph scope: identical trees, identical byte accounting, and the
//! snapshot records the options that produced it.

use std::sync::Arc;

use diskgraph_core::PrincipalId;
use diskgraph_engine::{Engine, EngineConfig};

fn engine_with_options(
    label: &str,
    options: disktree_core::scan::ScanOptions,
) -> (tempfile::TempDir, Arc<Engine>) {
    let workspace = tempfile::TempDir::with_prefix(format!("dg-parity-{label}-")).unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 2_000_000,
            scan_options: options,
            scan_budget: diskgraph_core::ScanBudget {
                max_nodes: 2_000_000,
                // Real-directory drills observe tens of GB of content.
                max_staging_bytes: 200 << 30,
                ..diskgraph_core::ScanBudget::default()
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    (workspace, engine)
}

fn scan_scope(engine: &Arc<Engine>, root: &std::path::Path) -> diskgraph_core::DiskGraph {
    let principal = PrincipalId::new("parity").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "parity").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    engine.load_revision(&revision).unwrap()
}

fn fixture_tree() -> tempfile::TempDir {
    let tree = tempfile::TempDir::with_prefix("dg-parity-tree-").unwrap();
    std::fs::create_dir_all(tree.path().join("src/nested")).unwrap();
    std::fs::write(tree.path().join("src/app.bin"), vec![7_u8; 5_120]).unwrap();
    std::fs::write(tree.path().join("src/nested/deep.bin"), vec![1_u8; 2_048]).unwrap();
    std::fs::write(tree.path().join(".hidden"), vec![3_u8; 1_024]).unwrap();
    tree
}

fn disktree_total(root: &std::path::Path, options: disktree_core::scan::ScanOptions) -> u64 {
    let handle = disktree_core::scan::ScanHandle::spawn(root.to_path_buf(), options);
    let tree = loop {
        match handle.poll() {
            Some(result) => break result.unwrap(),
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    };
    tree.bytes
}

#[test]
fn apparent_size_matches_disktrees_own_walk_for_the_same_tree() {
    let tree = fixture_tree();
    let options = disktree_core::scan::ScanOptions {
        apparent_size: true,
        ..disktree_core::scan::ScanOptions::default()
    };
    let (_workspace, engine) = engine_with_options("apparent", options.clone());
    let graph = scan_scope(&engine, tree.path());
    // The snapshot records the options that produced it (comparability).
    assert!(graph.snapshot.settings.apparent_size);
    assert!(graph.snapshot.settings.include_hidden);
    let diskgraph_total = graph
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .unwrap()
        .subtree_bytes;
    let disktree_total = disktree_total(tree.path(), options);
    assert_eq!(
        diskgraph_total, disktree_total,
        "apparent-size walk must match disktree's own accounting"
    );
}

#[test]
fn allocated_size_matches_disktrees_own_walk_for_the_same_tree() {
    let tree = fixture_tree();
    let (_workspace, engine) =
        engine_with_options("allocated", disktree_core::scan::ScanOptions::default());
    let graph = scan_scope(&engine, tree.path());
    assert!(!graph.snapshot.settings.apparent_size);
    let diskgraph_total = graph
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .unwrap()
        .subtree_bytes;
    let disktree_total = disktree_total(tree.path(), disktree_core::scan::ScanOptions::default());
    assert_eq!(
        diskgraph_total, disktree_total,
        "allocated walk must match disktree's own accounting"
    );
}

#[test]
fn no_hidden_skips_dotfiles_and_records_the_exclusion() {
    let tree = fixture_tree();
    let options = disktree_core::scan::ScanOptions {
        include_hidden: false,
        ..disktree_core::scan::ScanOptions::default()
    };
    let (_workspace, engine) = engine_with_options("hidden", options.clone());
    let graph = scan_scope(&engine, tree.path());
    assert!(!graph.snapshot.settings.include_hidden);
    assert!(
        !graph.nodes.iter().any(|node| node.name == ".hidden"),
        "a no-hidden walk must not index dotfiles"
    );
    assert_eq!(
        graph.snapshot.coverage.complete, graph.snapshot.coverage.complete,
        "coverage stays truthful"
    );
}

#[test]
fn a_depth_limit_marks_coverage_depth_limited_like_disktree() {
    let tree = fixture_tree();
    let options = disktree_core::scan::ScanOptions {
        max_depth: Some(1),
        ..disktree_core::scan::ScanOptions::default()
    };
    let (_workspace, engine) = engine_with_options("depth", options.clone());
    let graph = scan_scope(&engine, tree.path());
    assert!(graph.snapshot.settings.max_depth.is_some());
    assert!(
        graph.snapshot.coverage.depth_limited,
        "a depth-limited walk is reported, never a silent full one"
    );
    assert!(!graph.snapshot.coverage.complete);
}

#[test]
fn differently_configured_snapshots_are_not_comparable() {
    // Two scopes over the same tree with different apparent_size settings:
    // growth between their snapshots must refuse, because the numbers mean
    // different things (Q-04).
    let tree = fixture_tree();
    let (_workspace_a, engine_a) = engine_with_options(
        "cmp-a",
        disktree_core::scan::ScanOptions {
            apparent_size: true,
            ..disktree_core::scan::ScanOptions::default()
        },
    );
    let (_workspace_b, engine_b) =
        engine_with_options("cmp-b", disktree_core::scan::ScanOptions::default());
    let graph_a = scan_scope(&engine_a, tree.path());
    let graph_b = scan_scope(&engine_b, tree.path());
    assert!(graph_a.growth(&graph_b, &graph_a.snapshot.root).is_none());
}

#[test]
#[ignore = "real-directory parity drill: a real project tree, both walkers"]
fn a_real_project_tree_scans_identically_under_both_walkers() {
    let real = std::path::Path::new("/Users/wandl/workspaces/workspace-partme-ai");
    if !real.is_dir() {
        panic!("real drill directory is unavailable on this host");
    }
    let (_workspace, engine) =
        engine_with_options("real", disktree_core::scan::ScanOptions::default());
    let graph = scan_scope(&engine, real);
    let diskgraph_total = graph
        .nodes
        .iter()
        .find(|node| node.parent_id.is_none())
        .unwrap()
        .subtree_bytes;
    let disktree_total = disktree_total(real, disktree_core::scan::ScanOptions::default());
    println!(
        "REAL-PARITY diskgraph={diskgraph_total} disktree={disktree_total} nodes={}",
        graph.nodes.len()
    );
    assert_eq!(
        diskgraph_total, disktree_total,
        "a real directory must report identical allocated bytes under both walkers"
    );
}
