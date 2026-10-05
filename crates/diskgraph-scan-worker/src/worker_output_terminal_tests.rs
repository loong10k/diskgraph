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
