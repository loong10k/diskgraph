//! Directory comparison across two different roots.
//!
//! What this protects: `changes` refuses to compare two different roots, and
//! that refusal is right for it. Comparing two trees *is* the case where the
//! roots differ - a release build against a working copy - so it needs its own
//! path, and it has to reach the same answers a person would.

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
    let workspace = tempfile::TempDir::with_prefix("dg-compare-").unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 100_000,
            scan_budget: diskgraph_core::ScanBudget {
                max_nodes: 100_000,
                max_staging_bytes: 1 << 30,
                ..diskgraph_core::ScanBudget::default()
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    (workspace, engine)
}

/// Indexes `root` and returns its revision.
fn index(engine: &Arc<Engine>, root: &std::path::Path) -> String {
    let principal = PrincipalId::new("compare").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    // register_scope granted scope-local rights in the policy store, and the
    // authorizer handed in is a snapshot from before that: reusing it asks
    // for the job with stale rights and is refused.
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "compare").unwrap();
    engine.latest_revision(&scope).unwrap().unwrap()
}

/// Two trees that share relative paths and differ in the ways that matter.
fn pair() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let left = tempfile::TempDir::with_prefix("dg-cmp-left-").unwrap();
    let right = tempfile::TempDir::with_prefix("dg-cmp-right-").unwrap();
    for root in [left.path(), right.path()] {
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
    }
    // Identical on both sides, same size and timestamp is not guaranteed, so
    // the tolerance is what makes this row comparable rather than a
    // timestamp difference.
    std::fs::write(left.path().join("same.bin"), vec![1_u8; 4_096]).unwrap();
    std::fs::write(right.path().join("same.bin"), vec![1_u8; 4_096]).unwrap();
    std::fs::write(left.path().join("grown.bin"), vec![2_u8; 1_000]).unwrap();
    std::fs::write(right.path().join("grown.bin"), vec![2_u8; 8_000]).unwrap();
    std::fs::write(left.path().join("only-left.bin"), vec![3_u8; 512]).unwrap();
    std::fs::write(right.path().join("only-right.bin"), vec![4_u8; 512]).unwrap();
    // Same length, different bytes. A metadata-only comparison calls this the
    // same file, which is the honest limit of what it can say: promoting it to
    // "identical" needs the content grant, and a caller that skips a copy on
    // this row is relying on a comparison that never read either file.
    std::fs::write(left.path().join("src/lib.rs"), vec![5_u8; 256]).unwrap();
    std::fs::write(right.path().join("src/lib.rs"), vec![6_u8; 256]).unwrap();
    let left_root = left.path().to_path_buf();
    let right_root = right.path().to_path_buf();
    (left, right, left_root, right_root)
}

#[test]
fn two_unrelated_directories_compare_by_their_relative_paths() {
    let (_workspace, engine) = engine();
    let (_l, _r, left_root, right_root) = pair();
    let left_revision = index(&engine, &left_root);
    let right_revision = index(&engine, &right_root);
    assert_ne!(
        left_revision, right_revision,
        "two directories are two revisions"
    );

    let report = engine
        .compare_revisions(&left_revision, &right_revision, 4)
        .unwrap();
    assert_eq!(
        report.left_nodes, report.right_nodes,
        "same shape on purpose"
    );
    let verdict = |path: &str| {
        report
            .rows
            .iter()
            .find(|row| row.path == path)
            .unwrap_or_else(|| panic!("{path} is missing from {:?}", report.rows))
            .verdict
            .clone()
    };
    use diskgraph_core::Verdict;
    assert!(matches!(verdict("same.bin"), Verdict::Same { .. }));
    assert_eq!(
        verdict("grown.bin"),
        Verdict::Different {
            reason: diskgraph_core::DifferentReason::Size
        }
    );
    assert_eq!(verdict("only-left.bin"), Verdict::LeftOnly);
    assert_eq!(verdict("only-right.bin"), Verdict::RightOnly);
    assert!(matches!(verdict("src"), Verdict::Same { .. }));
    // Same length, different bytes, and the verdict says so by claiming the
    // weaker evidence: this row is only known to match on size and time.
    assert!(matches!(verdict("src/lib.rs"), Verdict::Same { .. }));

    assert_eq!(report.summary.left_only, 1);
    assert_eq!(report.summary.right_only, 1);
    assert_eq!(report.summary.different, 1);
    assert_eq!(report.summary.actionable(), 3, "three paths would move");
    assert_eq!(report.summary.unknown, 0);
}

#[test]
fn a_tree_compared_with_itself_has_nothing_actionable() {
    let (_workspace, engine) = engine();
    let (_l, _r, left_root, _right_root) = pair();
    let revision = index(&engine, &left_root);
    let report = engine.compare_revisions(&revision, &revision, 2).unwrap();
    assert_eq!(report.summary.actionable(), 0);
    assert!(report.rows.iter().all(|row| !row.verdict.is_difference()));
}

#[test]
fn a_report_says_what_it_compared_and_what_it_cost() {
    let (_workspace, engine) = engine();
    let (_l, _r, left_root, right_root) = pair();
    let left_revision = index(&engine, &left_root);
    let right_revision = index(&engine, &right_root);
    let report = engine
        .compare_revisions(&left_revision, &right_revision, 2)
        .unwrap();
    let json = report.to_json(Some(3));
    // Both roots are named, so a reader can tell a checkout against a checkout
    // from a comparison of two large trees without guessing.
    assert!(json["left"]["root"]["value"].is_string());
    assert!(json["right"]["root"]["value"].is_string());
    assert!(json["left"]["nodes"].as_u64().unwrap() > 0);
    assert_eq!(
        json["entries"],
        serde_json::json!(3),
        "the limit is honored"
    );
    // The summary is not cut by the row limit: it covers the whole compare.
    assert_eq!(
        json["summary"]["left_only"],
        serde_json::json!(report.summary.left_only)
    );
}
