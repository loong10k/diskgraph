//! PF-06 独立 pinned 扫描表征：固定八次，不重试到 winner 相同或不同。
//! 来源：真实 ScanHandle/aggregate_deduped；仅每次自身 Node→codec 的字段与顺序必须完全相同。
use diskgraph_disktree_core::{
    scan::{ScanHandle, ScanOptions},
    tree::Node,
};
use diskgraph_scan_worker::{
    FlatNodes, ProtocolLimits, ScanOptions as WireOptions, ScanProgress, read_tree,
    write_tree_with_limits,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

const RUNS: usize = 8;
const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn scan(root: &Path, options: ScanOptions) -> Result<(Node, ScanProgress), String> {
    let handle = ScanHandle::spawn(root.to_path_buf(), options);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(result) = handle.poll() {
            return result
                .map(|tree| (tree, ScanProgress::from_native(handle.progress.snapshot())))
                .map_err(|error| format!("pinned scan failed: {error:?}"));
        }
        if Instant::now() >= deadline {
            handle.cancel();
            return Err(
                "pinned scan exceeded qualification deadline; no physical-thread-join claim".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn observe(root: &Path, run: usize, options: ScanOptions) -> Result<Value, String> {
    let wire_options = WireOptions::from_native(&options);
    let (tree, progress) = scan(root, options)?;
    let expected = FlatNodes::new(&tree)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("flatten source: {error:?}"))?;
    let limits = ProtocolLimits {
        max_frame_bytes: 16_384,
        max_stream_bytes: 4_194_304,
        max_nodes: 1024,
        max_depth: 128,
    };
    let mut wire = Vec::new();
    let count = write_tree_with_limits(&mut wire, &tree, limits)
        .map_err(|error| format!("same-run codec write: {error:?}"))?;
    let decoded = read_tree(wire.as_slice(), limits)
        .map_err(|error| format!("same-run codec read: {error:?}"))?;
    let actual = FlatNodes::new(&decoded)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("flatten decoded: {error:?}"))?;
    let aliases: Vec<_> = expected
        .iter()
        .filter(|node| {
            matches!(
                node.name.as_str(),
                "ordinary.txt" | "hardlink.txt" | "linked-file"
            )
        })
        .collect();
    let winners: Vec<_> = aliases
        .iter()
        .filter(|node| node.own_bytes > 0)
        .map(|node| node.name.as_str())
        .collect();
    let inode = aliases.first().and_then(|node| node.inode);
    let one_identity = inode.is_some() && aliases.iter().all(|node| node.inode == inode);
    let sibling_order: Vec<_> = expected
        .iter()
        .filter(|node| node.parent == Some(0))
        .map(|node| node.name.as_str())
        .collect();
    Ok(json!({
        "run": run, "pin": PIN, "options": wire_options,
        "root": expected.first(), "progress": progress,
        "inode_group": aliases, "winner_names": winners,
        "group_bytes": aliases.iter().map(|node| node.bytes).sum::<u64>(),
        "sibling_order": sibling_order, "same_identity": one_identity,
        "same_run_all_fields_and_order_equal": expected == actual,
        "encoded_nodes": count, "encoded_bytes": wire.len(),
        "original_flat_nodes": expected, "decoded_flat_nodes": actual
    }))
}

#[test]
fn eight_real_pinned_runs_characterize_dedup_winners_and_preserve_each_tree() {
    let directory = tempfile::Builder::new()
        .prefix("diskgraph-pinned-characterization-")
        .tempdir()
        .unwrap();
    let root = directory.path().join("tree");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("ordinary.txt"), vec![1; 1337]).unwrap();
    std::fs::hard_link(root.join("ordinary.txt"), root.join("hardlink.txt")).unwrap();
    std::fs::write(root.join("nested/independent"), vec![2; 8193]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("ordinary.txt", root.join("linked-file")).unwrap();
    let options = ScanOptions {
        follow_links: true,
        ..ScanOptions::default()
    };
    let mut observations = Vec::new();
    let mut failures = Vec::new();
    // 固定次数即使前案失败也收集后案；不是重试到通过，也不把未观察到差异解释成确定性。
    for run in 0..RUNS {
        match observe(&root, run, options.clone()) {
            Ok(observation) => {
                eprintln!("PINNED_DEDUP_OBSERVATION {observation}");
                observations.push(observation);
            }
            Err(error) => {
                eprintln!(
                    "PINNED_DEDUP_OBSERVATION {}",
                    json!({"run":run,"error":error})
                );
                failures.push(error);
            }
        }
    }
    let winners: BTreeSet<_> = observations
        .iter()
        .filter_map(|row| row["winner_names"].get(0).and_then(Value::as_str))
        .collect();
    eprintln!(
        "PINNED_DEDUP_SUMMARY {}",
        json!({
            "requested_runs":RUNS,"completed_observations":observations.len(),
            "distinct_winner_names":winners,"different_winner_observed":winners.len()>1,
            "physical_thread_exit_certified":false
        })
    );
    assert!(
        failures.is_empty(),
        "all eight actual scans/codecs must qualify: {failures:?}"
    );
    assert_eq!(observations.len(), RUNS);
    let expected_alias_count = if cfg!(unix) { 3 } else { 2 };
    for observation in &observations {
        assert_eq!(
            observation["same_run_all_fields_and_order_equal"], true,
            "same-run codec must preserve every field/order: {observation}"
        );
        assert_eq!(
            observation["same_identity"], true,
            "real hardlink identity qualification"
        );
        assert_eq!(
            observation["inode_group"].as_array().unwrap().len(),
            expected_alias_count
        );
        assert_eq!(
            observation["winner_names"].as_array().unwrap().len(),
            1,
            "exactly one positive byte charge per real shared identity"
        );
        assert!(observation["group_bytes"].as_u64().unwrap() > 0);
        assert_eq!(
            observation["root"], observations[0]["root"],
            "root aggregate is independent of winning sibling name"
        );
        assert_eq!(observation["group_bytes"], observations[0]["group_bytes"]);
        assert_eq!(observation["progress"]["finished"], true);
        assert_eq!(observation["progress"]["cancelled"], false);
        assert_eq!(observation["progress"]["errors"], 0);
    }
    // 差异是否出现只作为真实观测输出，不将线程调度概率设为成功或失败门槛。
}
