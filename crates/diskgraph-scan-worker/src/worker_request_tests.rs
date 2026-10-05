//! helper v2 闭合输入回归；来源：真实 FrameReader 与 WorkerRequest serde 解码。
use crate::worker_request::WorkerRequest;
use crate::{FrameReader, ProtocolLimits};
use std::io::{self, Cursor};

fn decode(body: &[u8]) -> io::Result<Option<WorkerRequest>> {
    let mut wire = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    wire.extend_from_slice(body);
    FrameReader::new(
        Cursor::new(wire),
        ProtocolLimits {
            max_frame_bytes: 1024,
            max_stream_bytes: 2048,
            max_nodes: 1,
            max_depth: 0,
        },
    )
    .read_payload()
}

#[test]
fn clean_cancel_is_accepted_by_the_actual_bounded_reader() {
    assert!(matches!(
        decode(br#"{"type":"cancel"}"#).unwrap(),
        Some(WorkerRequest::Cancel {})
    ));
}

#[test]
fn unknown_cancel_fields_are_rejected_by_the_actual_bounded_reader() {
    let result = decode(br#"{"type":"cancel","extra":true}"#);
    match result {
        Err(error) => assert_eq!(error.kind(), io::ErrorKind::InvalidData),
        Ok(_) => panic!("closed v2 control accepted an unknown Cancel field"),
    }
}
