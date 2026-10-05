//! 公开类型化 Cancel 的闭合字段与流失败锁存；来源：PF-06 与真实 FrameReader。
use diskgraph_scan_worker::{Frame, FrameReader, ProtocolLimits, write_frame};
use std::io::{self, Cursor};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 1024,
        max_stream_bytes: 4096,
        max_nodes: 1,
        max_depth: 0,
    }
}

#[test]
fn public_cancel_preserves_constructor_wire_and_bounded_roundtrip() {
    let cancel = Frame::Cancel;
    assert_eq!(
        serde_json::to_vec(&cancel).unwrap(),
        br#"{"type":"cancel"}"#
    );
    let mut wire = Vec::new();
    write_frame(&mut wire, &cancel).unwrap();
    assert_eq!(&wire[4..], br#"{"type":"cancel"}"#);
    let mut reader = FrameReader::new(Cursor::new(wire), limits());
    assert_eq!(reader.read_frame().unwrap(), Some(Frame::Cancel));
    assert_eq!(reader.read_frame().unwrap(), None);
}

#[test]
fn public_cancel_unknown_field_is_invalid_and_latches_original_reader() {
    let body = br#"{"type":"cancel","extra":true}"#;
    let admitted = 4 + u64::try_from(body.len()).unwrap();
    let mut wire = u32::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    wire.extend_from_slice(body);
    write_frame(&mut wire, &Frame::Cancel).unwrap();
    let mut input = Cursor::new(wire);
    {
        let mut reader = FrameReader::new(&mut input, limits());
        let error = reader
            .read_frame()
            .expect_err("public Cancel accepted an unknown field");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(reader.bytes_read(), admitted);
        assert_eq!(
            reader.read_frame().unwrap_err().to_string(),
            "frame reader already failed"
        );
        assert_eq!(
            reader.expect_eof().unwrap_err().to_string(),
            "frame reader already failed"
        );
        assert_eq!(reader.bytes_read(), admitted);
    }
    assert_eq!(
        input.position(),
        admitted,
        "following clean frame must remain unread"
    );
}
