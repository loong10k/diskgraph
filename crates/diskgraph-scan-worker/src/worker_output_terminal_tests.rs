//! 最后一个真实 Node 后只剩终态预留时，End 不需要下一数据帧额度；来源：PF-06。
use crate::{
    ExecutionDecoder, ExecutionFrame, ExecutionOutcome, FlatNode, FrameReader, FrameWriter,
    ProtocolBudgetError, ProtocolLimits, WorkerFailure, worker_control::WorkerControl,
    worker_output::WorkerOutput,
};
use diskgraph_disktree_core::{scan::ScanProgress, tree::Node};
use std::io::{self, Read};
use std::sync::{Arc, mpsc};

/// 测试父端关闭之前保留实际控制 reader；来源：std::sync::mpsc 与 Read 的 EOF 语义。
struct GatedEof(mpsc::Receiver<()>);

impl Read for GatedEof {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        self.0
            .recv()
            .map_err(|_| io::Error::other("test release lost"))?;
        Ok(0)
    }
}

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 32768,
        max_nodes: 8,
        max_depth: 4,
    }
}

fn encoded<T: serde::Serialize>(payload: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    FrameWriter::new(&mut bytes, limits())
        .write_payload(payload)
        .unwrap();
    bytes
}

#[test]
fn end_consumes_reserved_space_after_last_node_without_an_extra_data_header() {
    let root = Node::directory("/root");
    let hello = encoded(&ExecutionFrame::<String, String>::Hello {
        version: 2,
        target: env!("DISKGRAPH_WORKER_TARGET").into(),
        pin: "158f9cc2f0b332194a3ffc5acec47760c99146d8".into(),
    });
    let node = encoded(&ExecutionFrame::<String, String>::Node {
        node: FlatNode::from_native(&root, 0, None, 0),
    });
    let reserve = encoded(&WorkerFailure::new(
        "output",
        ProtocolBudgetError::into_io("declared children exceed node limit"),
    ));
    let end = encoded(&ExecutionFrame::<String, String>::End { nodes: 1 });
    assert!(reserve.len() > end.len());
    let mut bound = limits();
    bound.max_stream_bytes = (hello.len() + node.len() + reserve.len()) as u64;
    let (release, gate) = mpsc::channel();
    let reader = FrameReader::new(GatedEof(gate), limits());
    let mut control = WorkerControl::start(reader, Arc::new(ScanProgress::default())).unwrap();
    let mut bytes = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut output = WorkerOutput::new(&mut bytes, bound);
        output.hello().unwrap();
        assert!(output.tree(&root, &control).is_ok());
    }));
    // 即使上面的真实输出断言失败，也先释放并 join 原控制 reader。
    control.mark_terminal();
    let released = release.send(());
    let joined = control.finish();
    released.unwrap();
    joined.unwrap();
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
    assert_eq!(bytes, [hello, node, end].concat());
    assert!(bytes.len() as u64 <= bound.max_stream_bytes);
    let mut decoder = ExecutionDecoder::new(
        bound,
        env!("DISKGRAPH_WORKER_TARGET"),
        "158f9cc2f0b332194a3ffc5acec47760c99146d8",
    )
    .unwrap();
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let (consumed, _) = decoder.push(remaining).unwrap();
        assert!(consumed > 0);
        remaining = &remaining[consumed..];
    }
    assert!(matches!(
        decoder.finish_eof().unwrap(),
        ExecutionOutcome::Tree(_)
    ));
}

/// 统计真实下游 Write 调用；固定树可验证批量传输而非仅检查缓冲类型。
struct CountedPipe {
    bytes: Arc<std::sync::Mutex<Vec<u8>>>,
    writes: Arc<std::sync::atomic::AtomicUsize>,
}
impl io::Write for CountedPipe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn actual_pipe_output_batches_nodes_and_flushes_hello_and_complete_terminal() {
    let mut root = Node::directory("root");
    root.children = (0..1000)
        .map(|n| Node::directory(format!("child-{n}")))
        .collect();
    let mut bound = limits();
    bound.max_nodes = 1001;
    bound.max_stream_bytes = 2 << 20;
    let bytes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (release, gate) = mpsc::channel();
    let reader = FrameReader::new(GatedEof(gate), limits());
    let mut control = WorkerControl::start(reader, Arc::new(ScanProgress::default())).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut output = WorkerOutput::for_pipe(
            CountedPipe {
                bytes: Arc::clone(&bytes),
                writes: Arc::clone(&writes),
            },
            bound,
        );
        output.hello().unwrap();
        assert!(!bytes.lock().unwrap().is_empty(), "Hello was not flushed");
        assert!(output.tree(&root, &control).is_ok());
        // 尚未 drop writer：成功 End 必须已经刷新，不借析构证明交付。
        let bytes = bytes.lock().unwrap();
        let mut decoder = ExecutionDecoder::new(
            bound,
            env!("DISKGRAPH_WORKER_TARGET"),
            "158f9cc2f0b332194a3ffc5acec47760c99146d8",
        )
        .unwrap();
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            let (consumed, _) = decoder.push(remaining).unwrap();
            assert!(consumed > 0);
            remaining = &remaining[consumed..];
        }
        assert!(matches!(
            decoder.finish_eof().unwrap(),
            ExecutionOutcome::Tree(_)
        ));
        eprintln!(
            "PIPE_BATCH nodes=1001 writes={} encoded_bytes={}",
            writes.load(std::sync::atomic::Ordering::SeqCst),
            bytes.len()
        );
        assert!(
            writes.load(std::sync::atomic::Ordering::SeqCst) < 100,
            "1001 nodes still require per-frame pipe writes"
        );
    }));
    control.mark_terminal();
    release.send(()).unwrap();
    control.finish().unwrap();
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

/// Hello 成功后使原下游永久失败，用于验证缓冲刷新失败不产生成功终态。
struct FailAfterHello(CountedPipe);
impl io::Write for FailAfterHello {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.writes.load(std::sync::atomic::Ordering::SeqCst) != 0 {
            // 注入固定原始错误码；不依赖目标平台的 libc 或声称真实管道错误验收。
            return Err(io::Error::from_raw_os_error(12345));
        }
        io::Write::write(&mut self.0, bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn buffered_terminal_flush_failure_preserves_pipe_error_and_latches_output() {
    let bytes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut output = WorkerOutput::for_pipe(
        FailAfterHello(CountedPipe {
            bytes: Arc::clone(&bytes),
            writes,
        }),
        limits(),
    );
    output.hello().unwrap();
    // Error 与正常 End 共用同一底层 flush；刷出失败必须保留原 errno 并锁存。
    let error = output
        .failure(&WorkerFailure::new(
            "scan_io",
            io::Error::new(io::ErrorKind::NotFound, "fixture"),
        ))
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(12345));
    assert!(
        output
            .failure(&WorkerFailure::new(
                "scan_io",
                io::Error::new(io::ErrorKind::NotFound, "fixture")
            ))
            .is_err()
    );
    let bytes = bytes.lock().unwrap();
    let mut decoder = ExecutionDecoder::new(
        limits(),
        env!("DISKGRAPH_WORKER_TARGET"),
        "158f9cc2f0b332194a3ffc5acec47760c99146d8",
    )
    .unwrap();
    let (consumed, _) = decoder.push(&bytes).unwrap();
    assert_eq!(consumed, bytes.len());
    assert!(
        decoder.finish_eof().is_err(),
        "failed flush became a complete terminal"
    );
}
