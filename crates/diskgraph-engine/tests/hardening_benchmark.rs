//! 隔离 release 夹具；旧完整加载与新窄读在同一 revision 上配对比较。
mod benchmark_support;

use diskgraph_core::{PrincipalId, QueryBudget};
use diskgraph_engine::{Engine, EngineConfig};
use std::{sync::Arc, time::Instant};

fn percentile(mut values: Vec<f64>, fraction: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * fraction).ceil() as usize]
}
fn timed(mut action: impl FnMut(), times: usize) -> serde_json::Value {
    let values: Vec<f64> = (0..times)
        .map(|_| {
            let start = Instant::now();
            action();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    serde_json::json!({"p50_ms":percentile(values.clone(),0.5),"p95_ms":percentile(values,0.95),"samples":times})
}
fn rss() -> u64 {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let raw = unsafe { usage.assume_init() }.ru_maxrss as u64;
            return if cfg!(target_os = "macos") {
                raw
            } else {
                raw * 1024
            };
        }
    }
    0
}
fn size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

#[test]
#[ignore = "release 20k/200k filesystem and paired query measurements"]
fn measure_isolated_release_fixtures() {
    if std::env::var_os("DG_MEASURE_CASE").is_none() {
        let outputs = tempfile::tempdir().unwrap();
        let mut all = Vec::new();
        for count in [20_000, 200_000, 300] {
            let output = outputs.path().join(format!("case-{count}.json"));
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "measure_isolated_release_fixtures",
                    "--ignored",
                    "--nocapture",
                ])
                .env("DG_MEASURE_CASE", count.to_string())
                .env("DISKGRAPH_BENCHMARK_OUTPUT", &output)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let cases: Vec<serde_json::Value> =
                serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
            for case in &cases {
                println!("{case}");
            }
            all.extend(cases);
        }
        if let Some(output) = std::env::var_os("DISKGRAPH_BENCHMARK_OUTPUT") {
            std::fs::write(output, serde_json::to_vec_pretty(&all).unwrap()).unwrap();
        }
        return;
    }
    let selected = std::env::var("DG_MEASURE_CASE")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut results = Vec::new();
    let paired = std::env::var_os("DG_MEASURE_ROOT").is_some();
    let cases = if paired {
        vec![(selected, std::env::var("DG_MEASURE_SHAPE").unwrap())]
    } else {
        vec![
            (20_000, "wide".into()),
            (200_000, "wide".into()),
            (300, "deep".into()),
        ]
    };
    for (count, shape) in cases {
        if selected != count {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = std::env::var_os("DG_MEASURE_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| dir.path().join("root"));
        if !paired {
            std::fs::create_dir(&root).unwrap();
            let mut parent = root.clone();
            for index in 0..count {
                if shape == "deep" {
                    parent.push("d");
                    std::fs::create_dir(&parent).unwrap();
                }
                std::fs::write(parent.join(format!("file-{index:06}")), [0; 32]).unwrap();
            }
        }
        let data = std::env::var_os("DG_MEASURE_DATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| dir.path().join("data"));
        let engine = Arc::new(
            Engine::open(EngineConfig {
                data_dir: data.clone(),
                max_nodes_per_scan: 1_000_000,
                ..EngineConfig::default()
            })
            .unwrap(),
        );
        let principal = PrincipalId::new("benchmark").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let storage_before = benchmark_support::storage(&data);
        let start = Instant::now();
        let outcome = engine.run_job(&job.job_id, "benchmark");
        if let Err(error) = &outcome {
            benchmark_support::ScanFailureDiagnostic::capture(
                &data,
                &job.job_id,
                scope.as_str(),
                principal.as_str(),
                "benchmark",
                error,
                start.elapsed(),
            )
            .emit();
        }
        // 后验诊断不接纳失败、不刷新原预算；保留同一个真实执行结果的原失败门禁。
        outcome.unwrap();
        let scan_seconds = start.elapsed().as_secs_f64();
        let scan_peak_rss = rss();
        let storage_after_scan = benchmark_support::storage(&data);
        let revision = engine.latest_revision(&scope).unwrap().unwrap();
        let qualification =
            paired.then(|| benchmark_support::qualify(&engine, &data, &revision, count, &shape));
        let narrow = timed(
            || {
                assert!(engine.revision_layer(&revision, 1, 20).unwrap().1.len() <= 20);
            },
            31,
        );
        let full = timed(
            || {
                let graph = engine.load_revision(&revision).unwrap();
                assert!(!graph.nodes.is_empty());
                std::hint::black_box(graph.children(1, 0, 20));
            },
            7,
        );
        let budget_tree = timed(
            || {
                let result = engine
                    .tree_view_bounded(&revision, 10, 0, QueryBudget::default())
                    .unwrap();
                assert!(!result.root.is_null());
            },
            11,
        );
        // The scanner does not assert that a root is rebuildable. Add one
        // explicit fixture fact so this measures a positive target, not only
        // the empty-evidence fast path.
        let snapshot_id = engine
            .revision_reader()
            .unwrap()
            .revision(&revision)
            .unwrap()
            .snapshot_id;
        rusqlite::Connection::open(data.join("diskgraph.sqlite"))
            .unwrap()
            .execute(
                "INSERT INTO evidence (snapshot_id, node_id, evidence_json) VALUES (?1, 1, ?2)",
                rusqlite::params![
                    snapshot_id,
                    serde_json::json!({
                        "node_id": 1, "relation": "rebuildable", "subject": "benchmark-fixture",
                        "source": "test", "observed_at_unix_ms": 1, "confidence": 100,
                    })
                    .to_string()
                ],
            )
            .unwrap();
        let candidate_narrow = timed(
            || {
                let result = engine
                    .review_candidates(
                        &revision,
                        1,
                        QueryBudget::default(),
                        &principal,
                        &engine.policy_authorizer().unwrap(),
                    )
                    .unwrap();
                assert_eq!(result.candidates.len(), 1);
            },
            11,
        );
        let candidate_full = timed(
            || {
                let graph = engine.load_revision(&revision).unwrap();
                std::hint::black_box(graph.candidates(1));
            },
            3,
        );
        let concurrent = std::thread::scope(|threads| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    let engine = &engine;
                    let revision = &revision;
                    threads.spawn(move || {
                        timed(
                            || {
                                engine.revision_layer(revision, 1, 20).unwrap();
                            },
                            31,
                        )
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        let storage_after_queries = benchmark_support::storage(&data);
        let storage =
            benchmark_support::phases(storage_before, storage_after_scan, storage_after_queries);
        let result = serde_json::json!({"native_qualification":qualification,"storage_phases":storage,"files":count,"shape":shape,"scan_seconds":scan_seconds,"process_scan_high_water_rss_bytes":scan_peak_rss,"process_total_high_water_rss_bytes":rss(),"database_bytes":size(&data.join("diskgraph.sqlite")),"wal_bytes":size(&data.join("diskgraph.sqlite-wal")),"before_full_revision_top20":full,"after_narrow_top20":narrow,"before_full_revision_candidates":candidate_full,"after_narrow_candidates":candidate_narrow,"budgeted_tree":budget_tree,"concurrent_4_readers":concurrent});
        println!("{}", result);
        results.push(result);
    }
    if let Some(output) = std::env::var_os("DISKGRAPH_BENCHMARK_OUTPUT") {
        std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    }
}
