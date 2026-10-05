//! 真实 Cargo helper 原 stdout 经同增量解码器；来源：std::process/std::io，非重编码 Value 回放。
//! 这里证明终帧握手、EOF 和实际 leader wait，未证明进程组空或受信安装。
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionEvent, ExecutionOutcome, FlatNodes, FrameWriter, ProtocolLimits,
    WorkerRequest,
};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn binary() -> PathBuf {
    let artifact = match option_env!("CARGO_BIN_EXE_diskgraph-scan-worker") {
        Some(value) => value,
        None => panic!("artifact qualification requires the actual Cargo worker binary"),
    };
    let binary = PathBuf::from(artifact);
    assert!(binary.is_absolute() && binary.is_file());
    binary
}

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 131072,
        max_nodes: 32,
        max_depth: 16,
    }
}

fn run(root: &Path, packet: &[u8]) -> (ExitStatus, ExecutionOutcome, Vec<ExecutionEvent>, u64) {
    let mut child = Command::new(binary())
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut source = child.stdout.take().unwrap();
    let diagnostics = child.stderr.take().unwrap();
    let write = input.write_all(packet);
    let reader = std::thread::spawn(move || {
        let mut input = Some(input);
        let mut decoder = ExecutionDecoder::new(limits(), env!("DISKGRAPH_WORKER_TARGET"), PIN)?;
        let mut seen = Vec::new();
        let mut buffer = [0_u8; 113];
        loop {
            let count = match source.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                let bytes = decoder.bytes_admitted();
                return Ok::<_, io::Error>((decoder.finish_eof()?, seen, bytes));
            }
            let mut remaining = &buffer[..count];
            while !remaining.is_empty() {
                let (consumed, event) = decoder.push(remaining)?;
                assert!(consumed > 0 && consumed <= remaining.len());
                remaining = &remaining[consumed..];
                if let Some(event) = event {
                    if matches!(event, ExecutionEvent::End { .. } | ExecutionEvent::Failed) {
                        // 父端先关闭 stdin，真实 helper 才能 join 控制读取并关闭 stdout。
                        drop(input.take());
                    }
                    seen.push(event);
                }
            }
        }
    });
    let diagnostics = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        diagnostics
            .take(65537)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut rescue = false;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            rescue = true;
            let _ = child.kill();
            break child.wait().expect("watchdog must reap child");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let decoded = reader.join();
    let diagnostics = diagnostics.join();
    // 先实际回收并 join 全部测试管道 owner，任何失败也不留下 helper。
    assert!(
        !rescue,
        "actual non-rescue leader wait required: {status:?}"
    );
    write.unwrap();
    let diagnostics = diagnostics.unwrap().unwrap();
    assert!(diagnostics.len() <= 65536);
    assert!(diagnostics.is_empty());
    let (outcome, events, bytes) = decoded.unwrap().unwrap();
    (status, outcome, events, bytes)
}

#[test]
fn actual_success_stdout_closes_control_on_end_then_delivers_tree_after_eof_and_wait() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("branch")).unwrap();
    std::fs::write(directory.path().join("branch/file"), b"native stream input").unwrap();
    let request = WorkerRequest::scan(directory.path(), &ScanOptions::default(), limits()).unwrap();
    let mut packet = Vec::new();
    FrameWriter::new(&mut packet, limits())
        .write_payload(&request)
        .unwrap();
    let (status, outcome, events, bytes) = run(directory.path(), &packet);
    assert!(status.success());
    assert!(bytes > 0 && bytes <= limits().max_stream_bytes);
    assert!(matches!(events.first(), Some(ExecutionEvent::Hello)));
    assert!(matches!(
        events.last(),
        Some(ExecutionEvent::End { nodes: 3 })
    ));
    assert!(events.iter().any(|event| matches!(event, ExecutionEvent::Progress(progress) if progress.finished && !progress.cancelled)));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ExecutionEvent::Failed))
    );
    match outcome {
        ExecutionOutcome::Tree(tree) => {
            let records = FlatNodes::new(&tree)
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(records.len(), 3);
            assert_eq!(records[1].name, "branch");
            assert_eq!(records[2].name, "file");
            assert_eq!(records[2].parent, Some(1));
        }
        ExecutionOutcome::Failure(_) => panic!("actual successful helper became failure"),
    }
}

#[test]
fn actual_prepare_failure_stdout_has_exact_io_protocol_failure_then_eof_and_failed_wait() {
    let directory = tempfile::tempdir().unwrap();
    let request = WorkerRequest::scan(directory.path(), &ScanOptions::default(), limits()).unwrap();
    let mut forged = serde_json::to_value(request).unwrap();
    forged["request"]["options"]["extra"] = serde_json::json!(true);
    let mut packet = Vec::new();
    FrameWriter::new(&mut packet, limits())
        .write_payload(&forged)
        .unwrap();
    let (status, outcome, events, bytes) = run(directory.path(), &packet);
    assert!(!status.success());
    assert!(bytes > 0 && bytes <= limits().max_stream_bytes);
    assert!(matches!(events.as_slice(), [ExecutionEvent::Failed]));
    match outcome {
        ExecutionOutcome::Failure(failure) => {
            assert_eq!(failure.code(), "protocol");
            assert_eq!(failure.io_kind(), io::ErrorKind::InvalidData);
            assert_eq!(failure.raw_os_error(), None);
            assert!(failure.message().contains("unknown field"));
        }
        ExecutionOutcome::Tree(_) => panic!("actual rejected helper delivered a tree"),
    }
}

// 仅缩小真实 Request 的节点或深度上限；原 run 的管道、decoder、EOF、wait 和救援断言不变。
fn actual_tree_budget_failure(bound: ProtocolLimits, expected_message: &str, emitted_nodes: usize) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("file"), b"actual quota scan input").unwrap();
    let request = WorkerRequest::scan(directory.path(), &ScanOptions::default(), bound).unwrap();
    let mut packet = Vec::new();
    FrameWriter::new(&mut packet, limits())
        .write_payload(&request)
        .unwrap();
    let (status, outcome, events, bytes) = run(directory.path(), &packet);
    assert_eq!(
        status.code(),
        Some(1),
        "actual helper must exit unsuccessfully"
    );
    assert!(bytes > 0 && bytes <= bound.max_stream_bytes);
    assert!(matches!(events.first(), Some(ExecutionEvent::Hello)));
    assert!(matches!(events.last(), Some(ExecutionEvent::Failed)));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ExecutionEvent::Failed))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ExecutionEvent::End { .. }))
    );
    assert!(events.iter().any(|event| matches!(event,
        ExecutionEvent::Progress(progress)
        if progress.finished && !progress.cancelled && progress.files == 1 && progress.errors == 0
    )), "real scanner must finish the one-file fixture before producer quota refusal");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ExecutionEvent::Node { .. }))
            .count(),
        emitted_nodes
    );
    match outcome {
        ExecutionOutcome::Failure(failure) => {
            assert_eq!(failure.code(), "output_budget");
            assert_eq!(failure.io_kind(), io::ErrorKind::InvalidData);
            assert_eq!(failure.raw_os_error(), None);
            assert_eq!(failure.message(), expected_message);
        }
        ExecutionOutcome::Tree(_) => panic!("actual producer quota failure delivered a tree"),
    }
}

#[test]
fn actual_node_budget_stdout_reports_typed_failure_after_scan_then_eof_and_failed_wait() {
    let mut bound = limits();
    bound.max_nodes = 1;
    actual_tree_budget_failure(bound, "declared children exceed node limit", 0);
}

#[test]
fn actual_depth_budget_stdout_reports_typed_failure_after_root_then_eof_and_failed_wait() {
    let mut bound = limits();
    bound.max_depth = 0;
    actual_tree_budget_failure(bound, "node preparation limit", 1);
}
