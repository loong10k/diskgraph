//! release 计划基准：父进程构造元数据，独立子进程测量实际计划和自身峰值 RSS。

use crate::PlanBuilder;
use crate::tests::{indexed, project, tree};
use diskgraph_core::{PrincipalId, ResourceLocator, ScopeId};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;

const CHILD_INPUT: &str = "DISKGRAPH_PLAN_METADATA_BENCH_INPUT";
const FIXTURE_MANIFEST: &str = "DISKGRAPH_PLAN_METADATA_BENCH_MANIFEST";

fn child_measure(input: Value) {
    if cfg!(debug_assertions) {
        panic!("measure release code only");
    }
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: input["data_dir"].as_str().unwrap().into(),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let scope = ScopeId::new(input["scope"].as_str().unwrap()).unwrap();
    let principal = PrincipalId::new(input["principal"].as_str().unwrap()).unwrap();
    let ids: Vec<u64> = input["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap())
        .collect();
    let builder = PlanBuilder::new(engine.clone());
    let mut samples = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        let plan = builder
            .build_trash_plan(&scope, &principal, &ids, 1 << 20)
            .unwrap();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(plan.items.len(), ids.len());
        assert_eq!(plan.expected_bytes, ids.len() as u64 * 64);
        assert_eq!(
            engine.control_store().unwrap().plan(&plan.plan_id).unwrap(),
            plan
        );
    }
    let first = samples[0];
    let raw = samples.clone();
    let mut warm = samples.split_off(1);
    warm.sort_by(f64::total_cmp);
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // 子进程尚未构造大图，RUSAGE_SELF 包含其打开 Engine/实际计划的峰值。
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    let usage = unsafe { usage.assume_init() };
    #[cfg(target_os = "macos")]
    let peak_rss_bytes = usage.ru_maxrss as u64;
    #[cfg(target_os = "linux")]
    let peak_rss_bytes = usage.ru_maxrss as u64 * 1024;
    println!(
        "DISKGRAPH_PLAN_METADATA_BENCH {}",
        json!({
            "metadata_rows": input["metadata_rows"], "selected": ids.len(),
            "samples_ms": raw, "first_ms": first,
            "warm_p50_ms": (warm[3] + warm[4]) / 2.0,
            "warm_p95_ms": warm[7], "peak_rss_bytes": peak_rss_bytes,
            "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
            "fixture": "synthetic metadata, selected real 64-byte files"
        })
    );
}

fn measure_fixture(input: &Value) {
    let root = std::path::Path::new(input["root"].as_str().unwrap());
    let before = tree(root);
    for selected in [1_usize, 16] {
        let mut child_input = input.clone();
        child_input["ids"] = Value::Array(input["ids"].as_array().unwrap()[..selected].to_vec());
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "tests::plan_query_tests::plan_query_benchmark::release_plan_metadata_scaling",
                "--nocapture",
            ])
            .env(CHILD_INPUT, child_input.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("1 passed; 0 failed"),
            "child did not execute: {stdout}"
        );
        let records: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with("DISKGRAPH_PLAN_METADATA_BENCH "))
            .collect();
        assert_eq!(records.len(), 1);
        println!("{}", records[0]);
        assert_eq!(tree(root), before);
    }
}

#[test]
#[ignore = "explicit release metadata scaling benchmark with fresh child processes"]
fn release_plan_metadata_scaling() {
    if let Ok(input) = std::env::var(CHILD_INPUT) {
        child_measure(serde_json::from_str(&input).unwrap());
        return;
    }
    if cfg!(debug_assertions) {
        panic!("run with --release");
    }
    let manifest = std::env::var_os(FIXTURE_MANIFEST).map(std::path::PathBuf::from);
    if let Some(path) = manifest.as_ref().filter(|path| path.exists()) {
        let fixtures: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(fixtures.len(), 2);
        for input in fixtures {
            measure_fixture(&input);
        }
        return;
    }
    let mut fixtures = Vec::new();
    for count in [20_000_usize, 200_000] {
        let mut project = project("plan-metadata-bench");
        for index in 0..16 {
            std::fs::write(
                project.root.join(format!("selected-{index:02}.bin")),
                [7_u8; 64],
            )
            .unwrap();
        }
        let (scope, principal) = indexed(&mut project);
        let original = project.engine.latest_revision(&scope).unwrap().unwrap();
        let mut graph = project.engine.load_revision(&original).unwrap();
        let ids: Vec<u64> = graph
            .nodes
            .iter()
            .filter(|node| node.name.starts_with("selected-"))
            .map(|node| node.id)
            .collect();
        assert_eq!(ids.len(), 16);
        let root_id = graph
            .nodes
            .iter()
            .find(|node| node.parent_id.is_none())
            .unwrap()
            .id;
        let next_id = graph.nodes.iter().map(|node| node.id).max().unwrap() + 1;
        let template = graph
            .nodes
            .iter()
            .find(|node| node.id == ids[0])
            .unwrap()
            .clone();
        let additional = count - graph.nodes.len();
        for offset in 0..additional {
            let mut node = template.clone();
            node.id = next_id + offset as u64;
            node.parent_id = Some(root_id);
            node.name = format!("metadata-{:06}", node.id);
            node.locator = ResourceLocator::NativePath(
                project.root.join(&node.name).to_str().unwrap().to_owned(),
            );
            node.direct_bytes = 0;
            node.subtree_bytes = 0;
            node.file_identity = None;
            graph.nodes.push(node);
        }
        assert_eq!(graph.nodes.len(), count);
        graph.snapshot.id = format!("metadata-bench-{count}");
        let graph_path = project._workspace.path().join("data/diskgraph.sqlite");
        let mut store = SqliteSnapshotStore::open(&graph_path).unwrap();
        let server = project.engine.server_id().unwrap();
        store.append_staging_nodes("bench", &graph.nodes).unwrap();
        store
            .publish_revision_owned(
                "bench",
                &graph,
                &format!("bench-revision-{count}"),
                1_900_000_000_000,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
        drop(store);
        drop(graph);
        let input = json!({
            "data_dir": project._workspace.path().join("data"), "root": project.root,
            "scope": scope.as_str(), "principal": principal.as_str(),
            "ids": ids, "metadata_rows": count
        });
        measure_fixture(&input);
        if manifest.is_some() {
            // 仅显式成对测量保留本次隔离夹具；后测复用同一图库和真实源文件。
            let _retained_fixture = project._workspace.keep();
            fixtures.push(input);
        }
    }
    if let Some(path) = manifest {
        std::fs::write(path, serde_json::to_vec_pretty(&fixtures).unwrap()).unwrap();
    }
}
