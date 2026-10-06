//! C07 growth over two revisions, read without loading either of them.
//!
//! The rule this file protects: answering "how much did this path grow"
//! costs the two rows it reports on. It used to cost both whole revisions in
//! memory, which is why the question was unanswerable on a large index.

// Linux 的扫描回归显式持有真实宿主；其他平台保留各自既有构造路径。
#[cfg(target_os = "linux")]
#[path = "support/native_scan_engine.rs"]
mod native_scan_engine;
#[cfg(not(target_os = "linux"))]
use diskgraph_engine::Engine;
#[cfg(target_os = "linux")]
use native_scan_engine::NativeScanEngine as Engine;

use std::sync::Arc;

use diskgraph_core::PrincipalId;
use diskgraph_engine::EngineConfig;

fn engine() -> (tempfile::TempDir, Arc<Engine>) {
    let workspace = tempfile::TempDir::with_prefix("dg-growth-").unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 2_000_000,
            scan_budget: diskgraph_core::ScanBudget {
                max_nodes: 2_000_000,
                max_staging_bytes: 4 << 30,
                ..diskgraph_core::ScanBudget::default()
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    (workspace, engine)
}

fn scope_of(engine: &Arc<Engine>, root: &std::path::Path) -> diskgraph_core::ScopeId {
    let principal = PrincipalId::new("growth").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    engine
        .register_scope(root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap()
}

fn rescan(engine: &Arc<Engine>, scope: &diskgraph_core::ScopeId) -> String {
    let principal = PrincipalId::new("growth").unwrap();
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.sync_scope(scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "growth").unwrap();
    engine.latest_revision(scope).unwrap().unwrap()
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let tree = tempfile::TempDir::with_prefix("dg-growth-tree-").unwrap();
    std::fs::create_dir_all(tree.path().join("cache/deep")).unwrap();
    std::fs::write(tree.path().join("cache/a.bin"), vec![1_u8; 4_096]).unwrap();
    std::fs::write(tree.path().join("cache/deep/b.bin"), vec![2_u8; 1_024]).unwrap();
    std::fs::write(tree.path().join("top.bin"), vec![3_u8; 8_192]).unwrap();
    let root = tree.path().to_path_buf();
    (tree, root)
}

#[test]
fn growth_of_a_nested_path_costs_two_rows_and_reports_the_delta() {
    let (_workspace, engine) = engine();
    let (_tree, root) = fixture();
    let scope = scope_of(&engine, &root);
    let before = rescan(&engine, &scope);

    // Grow one nested file: the subtree totals of its ancestors must move
    // with it, which is exactly the number a size report is about.
    std::fs::write(root.join("cache/deep/b.bin"), vec![2_u8; 512 * 1_024]).unwrap();
    let after = rescan(&engine, &scope);
    assert_ne!(before, after, "a rescan must publish a new revision");

    let path = std::path::Path::new("cache/deep");
    let growth = engine
        .growth_between(&before, &after, path)
        .unwrap()
        .expect("two complete scans of the same root are comparable");
    assert!(
        growth.delta_bytes > 0,
        "the file grew, so its directory did: {}",
        growth.delta_bytes
    );
    assert_eq!(growth.before.name, "deep");
    assert_eq!(growth.after.name, "deep");
    assert_eq!(
        growth.delta_bytes,
        i128::from(growth.after.subtree_bytes) - i128::from(growth.before.subtree_bytes),
        "the delta is the two subtree totals, not an estimate"
    );

    // The ancestors moved too, and each is answerable on its own.
    let cache = engine
        .growth_between(&before, &after, std::path::Path::new("cache"))
        .unwrap()
        .expect("comparable");
    let root_growth = engine
        .growth_between(&before, &after, std::path::Path::new(""))
        .unwrap()
        .expect("comparable");
    assert_eq!(
        cache.delta_bytes, growth.delta_bytes,
        "one file, one subtree"
    );
    assert!(root_growth.delta_bytes >= cache.delta_bytes);

    // A path that was never there is a miss, not a zero.
    assert!(
        engine
            .growth_between(&before, &after, std::path::Path::new("cache/absent"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_unchanged_path_reports_zero_rather_than_refusing() {
    let (_workspace, engine) = engine();
    let (_tree, root) = fixture();
    let scope = scope_of(&engine, &root);
    let before = rescan(&engine, &scope);
    let after = rescan(&engine, &scope);
    let growth = engine
        .growth_between(&before, &after, std::path::Path::new("cache"))
        .unwrap()
        .expect("comparable");
    assert_eq!(
        growth.delta_bytes, 0,
        "no change is an answer, not a missing one"
    );
}

#[test]
fn different_roots_are_never_compared() {
    let (_workspace, engine) = engine();
    // Both trees stay alive: the roots below are what the scopes point at.
    let (_first, first_root) = fixture();
    let (_second, second_root) = fixture();
    let first_scope = scope_of(&engine, &first_root);
    let second_scope = scope_of(&engine, &second_root);
    let first_revision = rescan(&engine, &first_scope);
    let second_revision = rescan(&engine, &second_scope);
    // A size delta across two different directories is a fiction, however
    // plausible the two numbers look.
    assert!(
        engine
            .growth_between(
                &first_revision,
                &second_revision,
                std::path::Path::new("cache")
            )
            .unwrap()
            .is_none(),
        "a delta between unrelated roots must be refused"
    );
}

#[test]
fn an_unknown_revision_is_an_error_not_a_growth_of_zero() {
    let (_workspace, engine) = engine();
    let (_tree, root) = fixture();
    let scope = scope_of(&engine, &root);
    let before = rescan(&engine, &scope);
    assert!(
        engine
            .growth_between(&before, "rev-does-not-exist", std::path::Path::new(""))
            .is_err(),
        "a missing revision must not read as 'nothing changed'"
    );
}

#[test]
fn walking_to_a_path_matches_loading_the_graph_and_finding_it() {
    // The narrow read has to agree with the in-memory answer it replaces.
    let (_workspace, engine) = engine();
    let (_tree, root) = fixture();
    let scope = scope_of(&engine, &root);
    let revision = rescan(&engine, &scope);
    let graph = engine.load_revision(&revision).unwrap();
    // The graph records the root it was given, and on macOS a temp directory
    // arrives through a symlink: compare against the recorded locator, not
    // the path this test happened to pass in.
    let recorded_root = match &graph.snapshot.root {
        diskgraph_core::ResourceLocator::NativePath(path) => std::path::PathBuf::from(path),
        other => panic!("a scan of a directory records a native path: {other:?}"),
    };
    for relative in ["", "cache", "cache/deep", "cache/deep/b.bin", "top.bin"] {
        let walked = engine
            .revision_node_at(&revision, std::path::Path::new(relative))
            .unwrap();
        let expected_locator = if relative.is_empty() {
            recorded_root.clone()
        } else {
            recorded_root.join(relative)
        };
        let expected = graph
            .nodes
            .iter()
            .find(|node| {
                node.locator
                    == diskgraph_core::ResourceLocator::NativePath(
                        expected_locator.to_string_lossy().into_owned(),
                    )
            })
            .cloned();
        assert_eq!(
            walked, expected,
            "the walked node must be the node the graph holds for {relative:?}"
        );
    }
}
