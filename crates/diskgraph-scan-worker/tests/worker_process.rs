//! PF-06 真实 worker 二进制资格及 pinned 扫描配对；缺产物仅为 artifact RED，不是取消/隔离证明。
mod worker_process {
    pub(super) mod alias_assertions;
    pub(super) mod worker_output;
}
use diskgraph_disktree_core::{
    scan::{ScanHandle, ScanOptions, ScanSnapshot},
    tree::{Metric, Node},
};
use diskgraph_scan_worker::{
    FlatNode, FlatNodes, NativePath, ScanOptions as WireOptions, ScanProgress,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use worker_process::worker_output::WorkerOutput;

const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn binary() -> PathBuf {
    let artifact = match option_env!("CARGO_BIN_EXE_diskgraph-scan-worker") {
        Some(value) => value,
        None => panic!(
            "artifact qualification: Cargo has not built the real diskgraph-scan-worker binary; this is not runtime cancellation or isolation RED"
        ),
    };
    let path = PathBuf::from(artifact);
    assert!(
        path.is_absolute() && path.is_file(),
        "fresh Cargo worker artifact must exist: {path:?}"
    );
    path
}

fn packet(value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    let mut bytes = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    bytes.extend(body);
    bytes
}

fn request(root: &Path, options: &ScanOptions) -> Value {
    json!({"type":"request", "version":2,
        "request":{"root":NativePath::from_path(root),"options":WireOptions::from_native(options)},
        "limits":{"max_frame_bytes":1048576,"max_stream_bytes":8388608,"max_nodes":10000,"max_depth":128}})
}

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::Builder::new()
        .prefix("diskgraph-worker-process-")
        .tempdir()
        .unwrap();
    let root = directory.path().join("tree");
    std::fs::create_dir_all(root.join("project/target/deep")).unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    std::fs::write(
        root.join("project/Cargo.toml"),
        b"[package]\nname='fixture'\n",
    )
    .unwrap();
    std::fs::write(root.join("project/target/object"), vec![7; 8193]).unwrap();
    std::fs::write(root.join("project/target/deep/tiny"), b"t").unwrap();
    std::fs::write(root.join(".hidden"), vec![2; 313]).unwrap();
    std::fs::write(root.join("ordinary.txt"), vec![1; 1337]).unwrap();
    let sparse = std::fs::File::create(root.join("sized.bin")).unwrap();
    sparse.set_len(2 * 1024 * 1024).unwrap();
    directory
}

fn add_aliases(root: &Path) {
    std::fs::hard_link(root.join("ordinary.txt"), root.join("hardlink.txt")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("ordinary.txt", root.join("linked-file")).unwrap();
}

fn pinned(root: &Path, options: ScanOptions) -> (Node, ScanSnapshot) {
    let handle = ScanHandle::spawn(root.to_path_buf(), options);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(result) = handle.poll() {
            return (
                result.expect("real pinned reference scan"),
                handle.progress.snapshot(),
            );
        }
        if Instant::now() >= deadline {
            handle.cancel();
            panic!("reference scanner exceeded fixture limit");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_success(output: &WorkerOutput, expected: &Node, progress: &ScanSnapshot) {
    assert!(
        output.status.success(),
        "actual exit={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout_bytes > 0);
    assert!(
        output.stderr.is_empty(),
        "successful helper must not emit unsolicited diagnostics"
    );
    let first = output.frames.first().expect("Hello required");
    assert_eq!(first["type"], "hello");
    assert_eq!(first["version"], 2);
    assert_eq!(first["pin"], PIN);
    assert!(
        first["target"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "target claim required; not executable trust proof"
    );
    let mut actual = Vec::new();
    let mut observed = Vec::new();
    let mut end = None;
    for frame in output.frames.iter().skip(1) {
        assert!(end.is_none(), "no frame after End");
        match frame["type"].as_str().unwrap() {
            "progress" => observed.push(
                serde_json::from_value::<ScanProgress>(frame["progress"].clone())
                    .unwrap()
                    .into_native(),
            ),
            "node" => {
                actual.push(serde_json::from_value::<FlatNode>(frame["node"].clone()).unwrap())
            }
            "end" => end = Some(frame["nodes"].as_u64().unwrap()),
            other => panic!("unexpected successful result phase: {other}: {frame}"),
        }
    }
    let expected: Vec<_> = FlatNodes::new(expected).collect::<Result<_, _>>().unwrap();
    assert_eq!(
        actual, expected,
        "every pinned field and child ordering must match"
    );
    assert_eq!(end, Some(expected.len() as u64));
    assert_eq!(
        observed.last(),
        Some(progress),
        "final actual progress preserves all seven fields/messages"
    );
    assert!(progress.finished && !progress.cancelled);
}

#[test]
fn real_worker_binary_artifact_is_available() {
    let _ = binary();
}

#[test]
fn real_worker_matches_pinned_fields_order_progress_and_all_scan_options() {
    let binary = binary();
    let directory = fixture();
    let root = directory.path().join("tree");
    let cases = [
        ScanOptions::default(),
        ScanOptions {
            apparent_size: true,
            ..ScanOptions::default()
        },
        ScanOptions {
            follow_links: true,
            ..ScanOptions::default()
        },
        ScanOptions {
            include_hidden: false,
            ..ScanOptions::default()
        },
        ScanOptions {
            one_filesystem: false,
            ..ScanOptions::default()
        },
        ScanOptions {
            max_depth: Some(0),
            ..ScanOptions::default()
        },
        ScanOptions {
            max_depth: Some(1),
            ..ScanOptions::default()
        },
        ScanOptions {
            dedup_hardlinks: false,
            ..ScanOptions::default()
        },
        ScanOptions {
            metric: Metric::Files,
            ..ScanOptions::default()
        },
        ScanOptions {
            apparent_size: true,
            follow_links: true,
            include_hidden: false,
            one_filesystem: false,
            max_depth: Some(2),
            dedup_hardlinks: false,
            metric: Metric::Files,
        },
    ];
    for options in cases {
        let (tree, progress) = pinned(&root, options.clone());
        let output = WorkerOutput::run(
            &binary,
            directory.path(),
            &packet(&request(&root, &options)),
        );
        assert_success(&output, &tree, &progress);
    }
}

#[test]
fn real_worker_preserves_not_found_and_non_directory_io_failures_without_end() {
    let binary = binary();
    let directory = fixture();
    for (path, expected_kind) in [
        (directory.path().join("missing"), "not_found"),
        (directory.path().join("tree/ordinary.txt"), "invalid_input"),
    ] {
        let output = WorkerOutput::run(
            &binary,
            directory.path(),
            &packet(&request(&path, &ScanOptions::default())),
        );
        assert!(
            !output.status.success(),
            "real io failure must exit nonzero"
        );
        assert!(
            output
                .frames
                .iter()
                .all(|frame| frame["type"] != "end" && frame["type"] != "node")
        );
        let failures: Vec<_> = output
            .frames
            .iter()
            .filter(|frame| frame["type"] == "error")
            .collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["code"], "scan_io");
        assert_eq!(failures[0]["io_kind"], expected_kind);
        assert!(
            failures[0].get("raw_os_error").is_some(),
            "optional native error field must be explicit"
        );
        assert!(
            failures[0]["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty())
        );
    }
}

#[test]
fn real_worker_rejects_bad_request_without_a_successful_tree() {
    let binary = binary();
    let directory = fixture();
    let valid = request(&directory.path().join("tree"), &ScanOptions::default());
    let mut wrong_version = valid.clone();
    wrong_version["version"] = json!(999);
    let mut forged = valid.clone();
    forged["authority"] = json!({"allow":true});
    let mut invalid_option = valid;
    invalid_option["request"]["options"]["metric"] = json!(255);
    for input in [wrong_version, forged, invalid_option] {
        let output = WorkerOutput::run(&binary, directory.path(), &packet(&input));
        assert!(!output.status.success());
        assert!(
            output
                .frames
                .iter()
                .all(|frame| frame["type"] != "node" && frame["type"] != "end")
        );
        let error = output
            .frames
            .iter()
            .find(|frame| frame["type"] == "error")
            .expect("bounded protocol Error required");
        assert_eq!(error["code"], "protocol");
    }
}

#[test]
fn real_worker_aliases_without_dedup_preserve_every_field_and_order() {
    let binary = binary();
    let directory = fixture();
    let root = directory.path().join("tree");
    add_aliases(&root);
    for follow_links in [false, true] {
        for apparent_size in [false, true] {
            let options = ScanOptions {
                follow_links,
                apparent_size,
                dedup_hardlinks: false,
                ..ScanOptions::default()
            };
            let (tree, progress) = pinned(&root, options.clone());
            let output = WorkerOutput::run(
                &binary,
                directory.path(),
                &packet(&request(&root, &options)),
            );
            assert_success(&output, &tree, &progress);
        }
    }
}

#[test]
fn real_worker_dedup_aliases_charge_one_identity_without_assuming_a_winner_name() {
    let binary = binary();
    let directory = fixture();
    let root = directory.path().join("tree");
    add_aliases(&root);
    // 该专案用真实 apparent length 获得独立单份大小，避免把任一上游 winner 名称作为 oracle。
    let single_size = std::fs::metadata(root.join("ordinary.txt")).unwrap().len();
    for follow_links in [false, true] {
        for metric in [Metric::Bytes, Metric::Files] {
            let options = ScanOptions {
                follow_links,
                apparent_size: true,
                metric,
                ..ScanOptions::default()
            };
            let (tree, progress) = pinned(&root, options.clone());
            let output = WorkerOutput::run(
                &binary,
                directory.path(),
                &packet(&request(&root, &options)),
            );
            worker_process::alias_assertions::assert_dedup_success(
                &output,
                &tree,
                &progress,
                metric,
                single_size,
            );
        }
    }
}
