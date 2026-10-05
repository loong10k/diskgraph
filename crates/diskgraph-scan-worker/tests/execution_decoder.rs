//! PF-06 v2 非阻塞输入合同；来源：真实帧字节，EOF 不表示 OS 进程退出。
mod execution_decoder {
    pub mod support;
}
use diskgraph_scan_worker::{ExecutionEvent, ExecutionOutcome, Frame, ScanProgress};
use execution_decoder::support::{decoder, hello, leaf, limits, one_node_stream, packet, root};
use std::io;

#[test]
fn every_header_and_body_split_preserves_one_terminal_event_then_requires_eof() {
    let stream = one_node_stream();
    for split in 0..=stream.len() {
        let mut input = decoder(limits());
        let mut events = execution_decoder::support::push_all(&mut input, &stream[..split]);
        events.extend(execution_decoder::support::push_all(
            &mut input,
            &stream[split..],
        ));
        assert!(matches!(
            events.as_slice(),
            [
                ExecutionEvent::Hello,
                ExecutionEvent::Node { nodes: 1 },
                ExecutionEvent::End { nodes: 1 }
            ]
        ));
        assert_eq!(input.bytes_admitted(), stream.len() as u64);
        match input.finish_eof().unwrap() {
            ExecutionOutcome::Tree(tree) => assert_eq!(tree.name.as_ref(), "/observed-root"),
            ExecutionOutcome::Failure(_) => panic!("successful complete stream became a failure"),
        }
        assert!(
            input.finish_eof().is_err(),
            "tree cannot be delivered twice"
        );
    }
}

#[test]
fn one_byte_fragments_and_coalesced_frames_consume_only_through_one_event() {
    let stream = one_node_stream();
    let mut input = decoder(limits());
    let mut events = Vec::new();
    for byte in &stream {
        let (consumed, event) = input.push(std::slice::from_ref(byte)).unwrap();
        assert_eq!(consumed, 1);
        events.extend(event);
    }
    assert_eq!(events.len(), 3);
    assert!(matches!(
        input.finish_eof().unwrap(),
        ExecutionOutcome::Tree(_)
    ));
    let mut input = decoder(limits());
    let (consumed, event) = input.push(&stream).unwrap();
    assert_eq!(
        consumed,
        hello().len(),
        "parent must observe each actual phase"
    );
    assert!(matches!(event, Some(ExecutionEvent::Hello)));
    assert_eq!(input.bytes_admitted(), hello().len() as u64);
}

#[test]
fn cumulative_stream_limit_includes_hello_and_all_tree_headers() {
    let stream = one_node_stream();
    let mut bound = limits();
    bound.max_stream_bytes = stream.len() as u64;
    let mut exact = decoder(bound);
    execution_decoder::support::push_all(&mut exact, &stream);
    assert!(matches!(
        exact.finish_eof().unwrap(),
        ExecutionOutcome::Tree(_)
    ));
    bound.max_stream_bytes -= 1;
    let mut input = decoder(bound);
    let prefix = stream.len() - packet(&Frame::End { nodes: 1 }).len();
    execution_decoder::support::push_all(&mut input, &stream[..prefix]);
    let error = input.push(&stream[prefix..prefix + 4]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(input.bytes_admitted(), prefix as u64);
    assert!(input.push(&stream[prefix..]).is_err());
    assert!(input.finish_eof().is_err());
}

#[test]
fn partial_header_body_and_missing_end_are_explicit_eof_failures() {
    for count in 1..4 {
        let mut input = decoder(limits());
        input.push(&[20, 0, 0][..count]).unwrap();
        assert_eq!(
            input.finish_eof().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(input.push(&hello()).is_err());
    }
    let mut input = decoder(limits());
    let mut short = 100_u32.to_le_bytes().to_vec();
    short.push(b'{');
    input.push(&short).unwrap();
    assert_eq!(input.bytes_admitted(), 104);
    assert_eq!(
        input.finish_eof().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        input.bytes_admitted(),
        104,
        "short body cannot refund admitted bytes"
    );
    let mut input = decoder(limits());
    execution_decoder::support::push_all(&mut input, &hello());
    assert_eq!(
        input.finish_eof().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn malformed_payload_cannot_be_skipped_or_continue_to_a_later_good_frame() {
    let mut input = decoder(limits());
    let malformed = packet(
        &serde_json::json!({"type":"hello","version":2,"target":"qualified-native-target","pin":"158f9cc2f0b332194a3ffc5acec47760c99146d8","extra":true}),
    );
    assert_eq!(
        input.push(&malformed).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(input.bytes_admitted(), malformed.len() as u64);
    assert!(input.push(&hello()).is_err());
    assert!(input.finish_eof().is_err());
    let mut input = decoder(limits());
    let malformed = [1_u32.to_le_bytes().as_slice(), b"{"].concat();
    let first = input.push(&malformed).unwrap_err();
    assert_eq!(first.kind(), io::ErrorKind::InvalidData);
    assert_eq!(input.bytes_admitted(), 5);
    assert_eq!(input.push(&hello()).unwrap_err().kind(), first.kind());
    assert_eq!(input.finish_eof().unwrap_err().kind(), first.kind());
}

#[test]
fn hello_version_target_pin_and_output_phase_must_match_the_same_execution() {
    for payload in [
        serde_json::json!({"type":"hello","version":1,"target":"qualified-native-target","pin":"158f9cc2f0b332194a3ffc5acec47760c99146d8"}),
        serde_json::json!({"type":"hello","version":2,"target":"other","pin":"158f9cc2f0b332194a3ffc5acec47760c99146d8"}),
        serde_json::json!({"type":"hello","version":2,"target":"qualified-native-target","pin":"other"}),
        serde_json::json!({"type":"cancel"}),
        serde_json::json!({"type":"unknown"}),
    ] {
        assert_eq!(
            decoder(limits())
                .push(&packet(&payload))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    let mut input = decoder(limits());
    assert_eq!(
        input
            .push(&packet(&Frame::Node { node: root(0) }))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let mut input = decoder(limits());
    execution_decoder::support::push_all(&mut input, &hello());
    assert_eq!(
        input.push(&hello()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn invalid_tree_parent_count_and_node_budget_never_deliver_partial_success() {
    for invalid in [
        Frame::End { nodes: 0 },
        Frame::Node {
            node: leaf(0, "unparented"),
        },
    ] {
        let mut input = decoder(limits());
        execution_decoder::support::push_all(&mut input, &hello());
        assert_eq!(
            input.push(&packet(&invalid)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(input.finish_eof().is_err());
    }
    let mut input = decoder(limits());
    execution_decoder::support::push_all(&mut input, &hello());
    execution_decoder::support::push_all(&mut input, &packet(&Frame::Node { node: root(1) }));
    assert_eq!(
        input
            .push(&packet(&Frame::End { nodes: 1 }))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let mut bound = limits();
    bound.max_nodes = 1;
    let mut input = decoder(bound);
    execution_decoder::support::push_all(&mut input, &hello());
    assert_eq!(
        input
            .push(&packet(&Frame::Node { node: root(1) }))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn progress_finished_and_end_are_not_clean_eof_or_os_exit_permits() {
    let mut input = decoder(limits());
    execution_decoder::support::push_all(&mut input, &hello());
    let progress = ScanProgress {
        files: 4,
        dirs: 2,
        bytes: 19,
        errors: 1,
        finished: true,
        cancelled: false,
        messages: vec!["real warning".into()],
    };
    let events = execution_decoder::support::push_all(
        &mut input,
        &packet(&Frame::Progress {
            progress: progress.clone(),
        }),
    );
    assert!(matches!(events.as_slice(), [ExecutionEvent::Progress(actual)] if actual == &progress));
    assert_eq!(
        input.finish_eof().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let mut input = decoder(limits());
    execution_decoder::support::push_all(&mut input, &one_node_stream());
    assert_eq!(
        input.push(b"x").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(
        input.finish_eof().is_err(),
        "terminal trailing byte is not ignorable"
    );
}

#[test]
fn failure_phase_has_one_terminal_and_only_protocol_can_precede_hello() {
    for code in ["control", "scan_io", "output", "cancelled"] {
        let failure = serde_json::json!({"type":"error","code":code,
            "io_kind":"interrupted","raw_os_error":null,"message":"actual failure"});
        let mut before = decoder(limits());
        assert_eq!(
            before.push(&packet(&failure)).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut after = decoder(limits());
        execution_decoder::support::push_all(&mut after, &hello());
        let (_, event) = after.push(&packet(&failure)).unwrap();
        assert!(matches!(event, Some(ExecutionEvent::Failed)));
        match after.finish_eof().unwrap() {
            ExecutionOutcome::Failure(actual) => {
                assert_eq!(actual.code(), code);
                assert_eq!(actual.io_kind(), io::ErrorKind::Interrupted);
                assert_eq!(actual.raw_os_error(), None);
                assert_eq!(actual.message(), "actual failure");
            }
            ExecutionOutcome::Tree(_) => panic!("failure phase delivered a tree"),
        }
        assert!(after.finish_eof().is_err());
    }
    let mut input = decoder(limits());
    let early = serde_json::json!({"type":"error","code":"protocol",
        "io_kind":"invalid_data","raw_os_error":null,"message":"rejected request"});
    let (_, event) = input.push(&packet(&early)).unwrap();
    assert!(matches!(event, Some(ExecutionEvent::Failed)));
    assert_eq!(
        input.push(&hello()).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(input.finish_eof().is_err());
}
