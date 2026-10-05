//! 同次真实 pinned 树与深树销毁；来源：PF-06，不声明 decoder 等同 child wait。
use diskgraph_disktree_core::{
    scan::{ScanHandle, ScanOptions},
    tree::Node,
};
use diskgraph_scan_worker::{
    ExecutionDecoder, ExecutionEvent, ExecutionOutcome, FlatNode, FlatNodes, Frame, FrameWriter,
    ProtocolLimits,
};
use std::time::{Duration, Instant};

const TARGET: &str = "qualified-native-target";
const PIN: &str = "158f9cc2f0b332194a3ffc5acec47760c99146d8";

fn feed(input: &mut ExecutionDecoder, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let (consumed, _) = input.push(bytes).unwrap();
        assert!(consumed > 0 && consumed <= bytes.len());
        bytes = &bytes[consumed..];
    }
}

#[test]
fn one_actual_pinned_scan_roundtrips_all_fields_and_sibling_order() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("branch/deep")).unwrap();
    std::fs::write(directory.path().join("first"), b"first observed file").unwrap();
    std::fs::write(directory.path().join("branch/deep/last"), b"last").unwrap();
    let scan = ScanHandle::spawn(directory.path().to_owned(), ScanOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    let tree = loop {
        if let Some(result) = scan.poll() {
            break result.unwrap();
        }
        if Instant::now() >= deadline {
            scan.cancel();
            panic!("single real pinned fixture did not complete");
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let bound = ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 131_072,
        max_nodes: 32,
        max_depth: 16,
    };
    let records = FlatNodes::new(&tree)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut wire = Vec::new();
    let mut writer = FrameWriter::new(&mut wire, bound);
    writer
        .write_frame(&Frame::Hello {
            version: 2,
            target: TARGET.into(),
            pin: PIN.into(),
        })
        .unwrap();
    for node in &records {
        writer
            .write_frame(&Frame::Node { node: node.clone() })
            .unwrap();
    }
    writer
        .write_frame(&Frame::End {
            nodes: records.len() as u64,
        })
        .unwrap();
    let mut input = ExecutionDecoder::new(bound, TARGET, PIN).unwrap();
    for chunk in wire.chunks(7) {
        feed(&mut input, chunk);
    }
    match input.finish_eof().unwrap() {
        ExecutionOutcome::Tree(decoded) => {
            let after = FlatNodes::new(&decoded)
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                after, records,
                "same observed tree must retain every field and order"
            );
        }
        ExecutionOutcome::Failure(_) => panic!("same real pinned tree became failure"),
    }
}

#[test]
fn deep_complete_tree_and_failed_partial_assembly_drop_without_recursive_stack() {
    std::thread::Builder::new()
        .stack_size(96 * 1024)
        .spawn(|| {
            let depth = 3000_u64;
            let bound = ProtocolLimits {
                max_frame_bytes: 8192,
                max_stream_bytes: 4 * 1024 * 1024,
                max_nodes: depth + 1,
                max_depth: depth,
            };
            let mut input = ExecutionDecoder::new(bound, TARGET, PIN).unwrap();
            let mut prefix = Vec::new();
            let mut writer = FrameWriter::new(&mut prefix, bound);
            writer
                .write_frame(&Frame::Hello {
                    version: 2,
                    target: TARGET.into(),
                    pin: PIN.into(),
                })
                .unwrap();
            for sequence in 0..=depth {
                let mut node = FlatNode::from_native(
                    &Node::directory("d"),
                    sequence,
                    sequence.checked_sub(1),
                    sequence,
                );
                node.child_count = u64::from(sequence < depth);
                writer.write_frame(&Frame::Node { node }).unwrap();
            }
            feed(&mut input, &prefix);
            let mut end = Vec::new();
            diskgraph_scan_worker::write_frame(&mut end, &Frame::End { nodes: depth + 1 }).unwrap();
            let (_, event) = input.push(&end).unwrap();
            assert!(matches!(event, Some(ExecutionEvent::End { nodes }) if nodes == depth + 1));
            match input.finish_eof().unwrap() {
                ExecutionOutcome::Tree(decoded) => {
                    let mut current: &Node = &decoded;
                    for _ in 0..depth {
                        assert_eq!(current.children.len(), 1);
                        current = &current.children[0];
                    }
                    assert!(current.children.is_empty());
                    drop(decoded);
                }
                ExecutionOutcome::Failure(_) => panic!("complete deep tree became failure"),
            }
            let mut failed = ExecutionDecoder::new(bound, TARGET, PIN).unwrap();
            feed(&mut failed, &prefix);
            assert_eq!(
                failed.finish_eof().unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof
            );
            drop(failed);
        })
        .unwrap()
        .join()
        .unwrap();
}
