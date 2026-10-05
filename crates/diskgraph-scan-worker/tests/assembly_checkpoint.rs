//! 真实平铺树组装的取消边界；来源：PF-06 父端组装检查点合同。
use diskgraph_disktree_core::tree::Node;
use diskgraph_scan_worker::{
    ExecutionDecodeError, ExecutionDecoder, ExecutionOutcome, FlatNode, Frame, FrameWriter,
    ProtocolLimits,
};

fn complete(count: u64, deep: bool) -> ExecutionDecoder {
    let limits = ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 16 * 1024 * 1024,
        max_nodes: count,
        max_depth: count,
    };
    let mut decoder = ExecutionDecoder::new(limits, "native", "pin").unwrap();
    let mut bytes = Vec::new();
    let mut writer = FrameWriter::new(&mut bytes, limits);
    writer
        .write_frame(&Frame::Hello {
            version: 2,
            target: "native".into(),
            pin: "pin".into(),
        })
        .unwrap();
    for sequence in 0..count {
        let parent = if deep {
            sequence.checked_sub(1)
        } else {
            (sequence > 0).then_some(0)
        };
        let mut node = FlatNode::from_native(
            &Node::directory(format!("n{sequence}").as_str()),
            sequence,
            parent,
            if deep {
                sequence
            } else {
                u64::from(sequence > 0)
            },
        );
        node.child_count = if deep {
            u64::from(sequence + 1 < count)
        } else if sequence == 0 {
            count - 1
        } else {
            0
        };
        writer.write_frame(&Frame::Node { node }).unwrap();
    }
    writer.write_frame(&Frame::End { nodes: count }).unwrap();
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let (used, _) = decoder.push(remaining).unwrap();
        remaining = &remaining[used..];
    }
    decoder
}

#[test]
fn wide_tree_preserves_sibling_order_with_repeated_original_checks() {
    let mut decoder = complete(4097, false);
    let mut checks = 0;
    let outcome = decoder
        .finish_eof_with_checkpoint(|| {
            checks += 1;
            Ok::<(), ()>(())
        })
        .unwrap();
    assert!(checks >= 32, "large tree must check within its traversal");
    let ExecutionOutcome::Tree(tree) = outcome else {
        panic!("complete tree")
    };
    assert_eq!(tree.children.len(), 4096);
    for (index, child) in tree.children.iter().enumerate() {
        assert_eq!(&*child.name, format!("n{}", index + 1));
    }
}

#[test]
fn checkpoint_failure_preserves_non_clone_reason_and_latches_decoder() {
    let mut decoder = complete(4097, false);
    #[derive(Debug)]
    struct Reason(u64);
    let original = Box::new(Reason(73));
    let address = std::ptr::from_ref(original.as_ref());
    let mut reason = Some(original);
    let result = decoder.finish_eof_with_checkpoint(|| Err(reason.take().unwrap()));
    match result {
        Err(ExecutionDecodeError::Checkpoint(reason)) => {
            assert_eq!(std::ptr::from_ref(reason.as_ref()), address);
            assert_eq!(reason.0, 73);
        }
        other => panic!("typed original reason lost: {other:?}"),
    }
    assert_eq!(
        decoder.finish_eof().unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn cancellation_after_partial_deep_merge_uses_iterative_disposal() {
    std::thread::Builder::new()
        .stack_size(96 * 1024)
        .spawn(|| {
            // Counting/reserving complete before these failures; at least 512 linked nodes exist.
            for stop in [46, 48, 50] {
                let mut decoder = complete(3001, true);
                let mut checks = 0;
                let result = decoder.finish_eof_with_checkpoint(|| {
                    checks += 1;
                    if checks == stop {
                        Err("original-cancel")
                    } else {
                        Ok(())
                    }
                });
                assert!(matches!(
                    result,
                    Err(ExecutionDecodeError::Checkpoint("original-cancel"))
                ));
                assert_eq!(checks, stop);
                drop(decoder);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn checkpoint_unwind_after_partial_deep_merge_disposes_without_recursive_stack() {
    std::thread::Builder::new()
        .stack_size(96 * 1024)
        .spawn(|| {
            let mut decoder = complete(3001, true);
            let mut checks = 0;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = decoder.finish_eof_with_checkpoint(|| {
                    checks += 1;
                    if checks == 48 {
                        panic!("original checkpoint panic");
                    }
                    Ok::<(), ()>(())
                });
            }));
            assert_eq!(checks, 48);
            let payload = outcome.unwrap_err();
            assert_eq!(
                payload.downcast_ref::<&str>(),
                Some(&"original checkpoint panic")
            );
            drop(decoder);
        })
        .unwrap()
        .join()
        .unwrap();
}
