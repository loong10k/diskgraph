//! PF-06 真实运行控制资格：固定夹具、固定三案、原总时钟，不重试到命中进度。
mod worker_control_process {
    pub(super) mod active_run;
    pub(super) mod control_action;
}
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{NativePath, ScanOptions as WireOptions};
use serde_json::json;
use std::path::PathBuf;
use worker_control_process::{active_run::ActiveRun, control_action::ControlAction};

fn exercise(action: ControlAction) {
    let binary = match option_env!("CARGO_BIN_EXE_diskgraph-scan-worker") {
        Some(binary) => PathBuf::from(binary),
        None => panic!(
            "actual Cargo helper artifact required; missing artifact is not a runtime control result"
        ),
    };
    assert!(binary.is_absolute() && binary.is_file());
    let directory = tempfile::Builder::new()
        .prefix("diskgraph-active-control-")
        .tempdir()
        .unwrap();
    let root = directory.path().join("tree");
    // 固定 16,384 个真实目录，各一个普通文件，共 32,769 个节点，少于旧夹具 32,897 个。
    // pinned 在 root 每累计 1,024 个目录就刷新真实进度，之后仍须逐目录读取文件。
    // 仅 child 的单线程 walk 安排提供早进度窗口，不是 OS barrier；未命中仍资格失败，绝不重试。
    // 准备仍在原运行时钟外，节点、深度、流与 30s 额度及所有行为断言保持原样。
    for branch in 0..16_384 {
        let path = root.join(format!("d{branch:05}"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("file"), b"x").unwrap();
    }
    let request = json!({"type":"request","version":2,
        "request":{"root":NativePath::from_path(&root),"options":WireOptions::from_native(&ScanOptions::default())},
        "limits":{"max_frame_bytes":1048576,"max_stream_bytes":33554432,"max_nodes":40000,"max_depth":4}});
    let body = serde_json::to_vec(&request).unwrap();
    let mut packet = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    packet.extend(body);
    let outcome = ActiveRun::run(&binary, directory.path(), packet, action);
    let result =
        outcome.unwrap_or_else(|error| panic!("actual {action:?} control run failed: {error:?}"));
    eprintln!(
        "WORKER_ACTIVE_CONTROL action={action:?} exit={:?} elapsed_us={} timed_out={} stderr_bytes={} witness={}",
        result.status,
        result.elapsed.as_micros(),
        result.timed_out,
        result.stderr.len(),
        result.output
    );
    assert!(
        !result.timed_out,
        "deadline kill is rescue, never successful control evidence"
    );
    assert!(
        !result.output["qualified"].is_null(),
        "no nonfinished real progress: unqualified, never skip/retry"
    );
    assert_eq!(result.output["action_before_deadline"], true);
    assert_eq!(result.output["action_completed"], true);
    assert_eq!(result.output["first"]["type"], "hello");
    assert_eq!(result.output["first"]["version"], 2);
    assert_eq!(
        result.output["first"]["pin"],
        "158f9cc2f0b332194a3ffc5acec47760c99146d8"
    );
    assert_eq!(
        result.output["ends"], 0,
        "no successful End after active control"
    );
    assert_eq!(result.output["errors"], 1);
    let (code, kind) = action.expected();
    assert_eq!(result.output["error"]["code"], code);
    assert_eq!(result.output["error"]["io_kind"], kind);
    assert!(result.output["error"].get("raw_os_error").is_some());
    assert_eq!(result.output["clean_stdout_eof"], true);
    assert!(
        !result.status.success(),
        "actual leader must exit unsuccessfully after control stop"
    );
    assert!(result.stderr.len() <= 65_536);
    assert!(
        result.stderr.is_empty(),
        "normal framed control failures require no fallback stderr"
    );
}

#[test]
fn cancel_after_real_active_progress_stops_worker_without_end() {
    exercise(ControlAction::Cancel);
}

#[test]
fn parent_eof_after_real_active_progress_stops_worker_without_end() {
    exercise(ControlAction::EndOfInput);
}

#[test]
fn invalid_control_after_real_active_progress_stops_worker_without_end() {
    exercise(ControlAction::InvalidControl);
}
