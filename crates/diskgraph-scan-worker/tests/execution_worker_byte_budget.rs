//! PF-06 真实 Cargo helper 的字节终态余额；来源：std::process 管道与 v2 增量协议。
//! 只证明实际 leader wait，不把终帧、EOF 或本测试当作受信安装及进程组空许可。
use diskgraph_disktree_core::scan::ScanOptions;
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionEvent, ExecutionFrame, ExecutionOutcome, FrameWriter,
    ProtocolLimits, WorkerInputLimits, WorkerIoKind, WorkerRequest,
};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 131072,
        max_nodes: 128,
        max_depth: 16,
    }
}

fn encoded<T: serde::Serialize>(payload: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    FrameWriter::new(&mut bytes, limits())
        .write_payload(payload)
        .unwrap();
    bytes
}

fn qualify_terminal_capacity(bound: ProtocolLimits) {
    let hello = encoded(&ExecutionFrame::<String, String>::Hello {
        version: 2,
        target: env!("DISKGRAPH_WORKER_TARGET").into(),
        pin: PIN.into(),
    });
    for message in ["frame byte limit", "stream byte limit"] {
        let failure = encoded(&ExecutionFrame::Error {
            code: "output_budget",
            io_kind: WorkerIoKind::InvalidData,
            raw_os_error: None,
            message,
        });
        assert!(hello.len() as u64 - 4 <= bound.max_frame_bytes);
        assert!(failure.len() as u64 - 4 <= bound.max_frame_bytes);
        assert!((hello.len() + failure.len()) as u64 <= bound.max_stream_bytes);
    }
}

fn run(root: &Path, bound: ProtocolLimits) -> (i32, ExecutionOutcome, Vec<ExecutionEvent>, u64) {
    qualify_terminal_capacity(bound);
    let request = WorkerRequest::scan(root, &ScanOptions::default(), bound).unwrap();
    let mut packet = Vec::new();
    FrameWriter::new(&mut packet, WorkerInputLimits::protocol_limits())
        .write_payload(&request)
        .unwrap();
    let binary = match option_env!("CARGO_BIN_EXE_diskgraph-scan-worker") {
        Some(binary) => Path::new(binary),
        None => panic!("actual Cargo worker artifact is required"),
    };
    assert!(binary.is_absolute() && binary.is_file());
    // 单次原时钟包含启动和输入；不会在错误或清理时重新开始 30 秒窗口。
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut child = Command::new(binary)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let write = input.write_all(&packet);
    let reader = std::thread::spawn(move || {
        let mut input = Some(input);
        let mut decoder = ExecutionDecoder::new(bound, env!("DISKGRAPH_WORKER_TARGET"), PIN)?;
        let mut events = Vec::new();
        let mut buffer = [0; 113];
        let mut actual_bytes = 0_u64;
        loop {
            let count = match output.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                let admitted = decoder.bytes_admitted();
                let outcome = decoder.finish_eof()?;
                if admitted != actual_bytes {
                    return Err(io::Error::other("actual output and decoder ledger differ"));
                }
                return Ok::<_, io::Error>((outcome, events, actual_bytes));
            }
            actual_bytes = actual_bytes.checked_add(count as u64).unwrap();
            if actual_bytes > bound.max_stream_bytes {
                return Err(io::Error::other(
                    "actual output exceeded original stream allowance",
                ));
            }
            let mut remaining = &buffer[..count];
            while !remaining.is_empty() {
                let (consumed, event) = decoder.push(remaining)?;
                assert!(consumed > 0 && consumed <= remaining.len());
                remaining = &remaining[consumed..];
                if let Some(event) = event {
                    if matches!(event, ExecutionEvent::End { .. } | ExecutionEvent::Failed) {
                        drop(input.take());
                    }
                    events.push(event);
                }
            }
        }
    });
    let diagnostics = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(65537).read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut rescue = false;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            rescue = true;
            let _ = child.kill();
            break child
                .wait()
                .expect("watchdog must consume actual leader wait");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let decoded = reader.join();
    let diagnostics = diagnostics.join();
    // 全部本案进程/管道 owner 回收后才报告失败，不用固定 stderr 文本推断 Budget。
    let diagnostics = diagnostics.unwrap().unwrap();
    eprintln!(
        "actual_byte_budget status={status:?} rescue={rescue} stderr={:?}",
        String::from_utf8_lossy(&diagnostics)
    );
    assert!(
        !rescue,
        "terminal Error must permit ordinary EOF and original wait"
    );
    write.unwrap();
    assert!(diagnostics.len() <= 65536);
    assert!(
        diagnostics.is_empty(),
        "typed failure must fit the original output allowance"
    );
    let (outcome, events, bytes) = decoded.unwrap().unwrap();
    (
        status.code().expect("normal helper exit code"),
        outcome,
        events,
        bytes,
    )
}

fn fixture(files: usize) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..files {
        let name = format!("{index:03}{}", "x".repeat(189));
        std::fs::write(directory.path().join(name), b"actual byte quota fixture").unwrap();
    }
    directory
}

fn assert_budget(root: &Path, bound: ProtocolLimits, files: u64) {
    let (code, outcome, events, bytes) = run(root, bound);
    assert_eq!(code, 1);
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
        if progress.finished && !progress.cancelled && progress.files == files && progress.errors == 0
    )), "actual scanner must finish this fixed fixture before result byte refusal");
    match outcome {
        ExecutionOutcome::Failure(failure) => {
            assert_eq!(failure.code(), "output_budget");
            assert_eq!(failure.io_kind(), io::ErrorKind::InvalidData);
            assert_eq!(failure.raw_os_error(), None);
            assert!(!failure.message().is_empty());
        }
        ExecutionOutcome::Tree(_) => panic!("real byte quota failure cannot deliver a tree"),
    }
}

#[test]
fn actual_frame_byte_quota_preserves_bounded_typed_terminal_and_wait() {
    let directory = fixture(1);
    let mut bound = limits();
    bound.max_frame_bytes = 256;
    assert_budget(directory.path(), bound, 1);
}

#[test]
fn actual_stream_byte_quota_preserves_bounded_typed_terminal_and_wait() {
    let directory = fixture(32);
    let mut bound = limits();
    bound.max_stream_bytes = 2048;
    assert_budget(directory.path(), bound, 32);
}

#[test]
fn ample_original_bytes_deliver_complete_same_fixture_without_budget_error() {
    let directory = fixture(32);
    let (code, outcome, events, bytes) = run(directory.path(), limits());
    assert_eq!(code, 0);
    assert!(bytes > 2048 && bytes <= limits().max_stream_bytes);
    assert!(matches!(
        events.last(),
        Some(ExecutionEvent::End { nodes: 33 })
    ));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ExecutionEvent::Failed))
    );
    match outcome {
        ExecutionOutcome::Tree(tree) => {
            assert_eq!(tree.children.len(), 32);
            for index in 0..32 {
                let name = format!("{index:03}{}", "x".repeat(189));
                assert!(tree.children.iter().any(|node| node.name.as_ref() == name));
            }
        }
        ExecutionOutcome::Failure(_) => panic!("ample actual scan unexpectedly failed"),
    }
}
