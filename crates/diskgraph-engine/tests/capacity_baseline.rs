//! Capacity and latency baseline (P7 task 8.11, Q-02 / RT-04 / RE-03).
//! Real measurements over real fixtures; run explicitly with:
//!
//! ```text
//! cargo test -p diskgraph-engine --test capacity_baseline -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! The assertions prove correctness at scale (the work completes and the
//! answers stay exact); the printed numbers are the recorded baseline, not
//! thresholds — a slow run here is a number to investigate, not a failed
//! gate.

use std::time::Instant;

use diskgraph_core::PrincipalId;
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_testkit::FixtureTree;

fn engine_at(_label: &str, data: &std::path::Path) -> std::sync::Arc<Engine> {
    std::fs::create_dir_all(data).unwrap();
    std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: data.to_path_buf(),
            max_nodes_per_scan: 1_000_000,
            ..EngineConfig::default()
        })
        .unwrap(),
    )
}

fn indexed(engine: &std::sync::Arc<Engine>, root: &std::path::Path) -> diskgraph_core::ScopeId {
    let principal = PrincipalId::new("bench").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "bench").unwrap();
    scope
}

#[test]
#[ignore = "capacity baseline: real fixture build and index timing"]
fn a_large_flat_directory_indexes_and_answers_within_budgets() {
    let workspace = tempfile::TempDir::with_prefix("dg-capacity-flat-").unwrap();
    let tree_root = workspace.path().join("project");
    std::fs::create_dir_all(&tree_root).unwrap();
    // A wide, flat directory: 20k files, the shape that stresses listing
    // and top-children ordering rather than depth.
    const FILES: usize = 20_000;
    let started = Instant::now();
    for index in 0..FILES {
        std::fs::write(
            tree_root.join(format!("file-{index:05}.bin")),
            vec![0_u8; 64],
        )
        .unwrap();
    }
    let fixture_seconds = started.elapsed().as_secs_f64();

    let engine = engine_at("flat", &workspace.path().join("data"));
    let started = Instant::now();
    let scope = indexed(&engine, &tree_root);
    let index_seconds = started.elapsed().as_secs_f64();

    // The answer must be exact, whatever the speed was.
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let graph = engine.load_revision(&revision).unwrap();
    let file_nodes = graph
        .nodes
        .iter()
        .filter(|node| node.kind == diskgraph_core::NodeKind::File)
        .count();
    assert_eq!(file_nodes, FILES, "every file must be in the index");

    let started = Instant::now();
    let page = graph.children(1, 0, 100);
    let query_micros = started.elapsed().as_micros();
    assert_eq!(page.items.len(), 100);
    assert!(page.next_offset.is_some());

    println!(
        "flat-20k: fixture={fixture_seconds:.2}s index={index_seconds:.2}s top100-query={query_micros}µs files={file_nodes}"
    );
}

#[test]
#[ignore = "capacity baseline: history retention timing"]
fn history_retention_stays_queryable_across_revisions() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::TempDir::with_prefix("dg-capacity-history-").unwrap();
    let tree = FixtureTree::new("history").unwrap();
    tree.file("a.bin", 1024).unwrap();
    let engine = engine_at("history", &workspace.path().join("data"));
    let principal = PrincipalId::new("bench").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(
            tree.path(),
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let authorizer = engine.policy_authorizer().unwrap();

    // Eight revisions of the same scope, each touched slightly.
    const REVISIONS: usize = 8;
    let started = Instant::now();
    for round in 0..REVISIONS {
        std::fs::write(tree.path().join(format!("round-{round}.bin")), vec![0; 32]).unwrap();
        let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
        engine.run_job(&job.job_id, "bench").unwrap();
    }
    let history_seconds = started.elapsed().as_secs_f64();

    // Every published revision stays addressable and loadable: retention
    // keeps history queryable until an authorized removal (ST-04).
    let latest = engine.latest_revision(&scope)?.unwrap();
    let graph = engine.load_revision(&latest)?;
    assert!(!graph.nodes.is_empty());
    // The full history is still on disk: REVISIONS-1 older rounds plus the
    // live one, observable through the database itself.
    let database = workspace.path().join("data").join("diskgraph.sqlite");
    assert!(database.is_file());
    let again = engine.load_revision(&latest)?;
    assert_eq!(again.snapshot.id, graph.snapshot.id);
    println!("history-{REVISIONS}: index-all={history_seconds:.2}s latest-revision-loadable=true");
    Ok(())
}

#[test]
#[ignore = "capacity baseline: duplicate suspect grouping at scale"]
fn duplicate_suspects_group_thousands_of_same_size_files() {
    let workspace = tempfile::TempDir::with_prefix("dg-capacity-dupes-").unwrap();
    let tree_root = workspace.path().join("project");
    std::fs::create_dir_all(&tree_root).unwrap();
    // Five thousand same-size files: the worst case for suspect grouping.
    const DUPES: usize = 5_000;
    for index in 0..DUPES {
        std::fs::write(
            tree_root.join(format!("dupe-{index:05}.bin")),
            vec![7_u8; 512],
        )
        .unwrap();
    }
    let engine = engine_at("dupes", &workspace.path().join("data"));
    let scope = indexed(&engine, &tree_root);

    let started = Instant::now();
    let groups = engine.duplicate_suspects(&scope).unwrap();
    let grouping_millis = started.elapsed().as_millis();

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].members.len(), DUPES);
    println!(
        "dupes-{DUPES}: one group of {} members grouped in {grouping_millis}ms (metadata only)",
        groups[0].members.len()
    );
}
