//! 隔离元数据的关系查询阶段采样；不编入生产，也不证明原生扫描或总体性能验收。
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Instant;

type Recording = (Instant, Vec<(&'static str, f64)>);
thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
    static DETAILS: RefCell<Vec<(&'static str, f64)>> = const { RefCell::new(Vec::new()) };
}

/// 记录终检子阶段，不移动外层阶段时钟。参数：label 为阶段，start 为起点；返回：无。
pub(super) fn detail(label: &'static str, start: Instant) {
    RECORDING.with(|recording| {
        if recording.borrow().is_some() {
            DETAILS.with(|details| {
                details
                    .borrow_mut()
                    .push((label, start.elapsed().as_secs_f64() * 1000.0))
            });
        }
    });
}

/// 参数：label 为固定阶段名；返回：无，仅启用的当前测试线程记录阶段耗时。
pub(super) fn mark(label: &'static str) {
    RECORDING.with(|recording| {
        if let Some((previous, samples)) = recording.borrow_mut().as_mut() {
            let now = Instant::now();
            samples.push((label, now.duration_since(*previous).as_secs_f64() * 1000.0));
            *previous = now;
        }
    });
}

#[test]
#[ignore = "isolated release metadata profiling; not native scan qualification"]
fn isolated_positive_candidate_phase_costs() {
    let (_dir, engine, principal, scope, original) =
        crate::relation_request_tests::published_authorization_fixture();
    let revision = "phase-diagnostic-revision";
    let server = engine.server_id().unwrap();
    {
        let mut store = engine.graph().unwrap();
        let mut graph = store.load_revision(original).unwrap();
        graph.snapshot.id = "phase-diagnostic-snapshot".into();
        graph.nodes[0].subtree_bytes = 256;
        graph.nodes[0].files = 1;
        let mut child = graph.nodes[0].clone();
        child.id = 2;
        child.parent_id = Some(1);
        child.name = "synthetic-file.bin".into();
        child.kind = diskgraph_core::NodeKind::File;
        child.direct_bytes = 256;
        child.directories = 0;
        child.locator = serde_json::from_value(serde_json::json!({
            "type":"native_path", "value":_dir.path().join("synthetic-file.bin")
        }))
        .unwrap();
        graph.nodes.push(child);
        graph.evidence = serde_json::from_value(serde_json::json!([{
            "node_id":1,"relation":"rebuildable","subject":"synthetic-phase-fixture",
            "source":"test","observed_at_unix_ms":1,"confidence":100
        }]))
        .unwrap();
        store
            .append_staging_nodes("phase-diagnostic-job", &graph.nodes)
            .unwrap();
        store
            .publish_revision_owned(
                "phase-diagnostic-job",
                &graph,
                revision,
                2,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
    }
    let mut phases: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let expected = [
        "policy_load",
        "reader_open",
        "initial_authorization",
        "data_read",
        "terminal_before_encode",
        "encode",
        "terminal_after_encode",
    ];
    for _ in 0..100 {
        DETAILS.with(|details| details.borrow_mut().clear());
        let request_started = Instant::now();
        RECORDING.with(|recording| *recording.borrow_mut() = Some((Instant::now(), Vec::new())));
        let policy = engine.policy_authorizer().unwrap();
        mark("policy_load");
        let answer = engine
            .review_candidates(
                revision,
                1,
                diskgraph_core::QueryBudget::default(),
                &principal,
                &policy,
            )
            .unwrap();
        let request_elapsed = request_started.elapsed().as_secs_f64() * 1000.0;
        phases
            .entry("request_wall")
            .or_default()
            .push(request_elapsed);
        assert_eq!(
            answer.candidates.len(),
            1,
            "must exercise positive evidence selection"
        );
        let (_, recorded) = RECORDING.with(|recording| recording.borrow_mut().take().unwrap());
        assert_eq!(recorded.len(), 7, "all original phases must execute");
        assert_eq!(
            recorded.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            expected
        );
        phases
            .entry("instrumented_phases_total")
            .or_default()
            .push(recorded.iter().map(|(_, elapsed)| elapsed).sum());
        for (label, elapsed) in recorded {
            phases.entry(label).or_default().push(elapsed);
        }
        let details = DETAILS.with(|details| std::mem::take(&mut *details.borrow_mut()));
        assert_eq!(
            details.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            [
                "terminal_reader_open",
                "terminal_ownership_sql",
                "terminal_reader_open",
                "terminal_ownership_sql"
            ]
        );
        for (label, elapsed) in details {
            phases.entry(label).or_default().push(elapsed);
        }
    }
    let output: BTreeMap<_, _> = phases
        .into_iter()
        .map(|(label, mut samples)| {
            samples.sort_by(f64::total_cmp);
            let count = samples.len();
            (
                label,
                serde_json::json!({"p50_ms":samples[count / 2 - 1], "p95_ms":samples[count * 95 / 100 - 1], "samples":count}),
            )
        })
        .collect();
    println!(
        "{}",
        serde_json::json!({"fixture":"synthetic two-node positive evidence",
        "profile":if cfg!(debug_assertions) { "debug" } else { "release" }, "os":std::env::consts::OS,
        "native_scan_qualification":false,"phases":output})
    );
}
