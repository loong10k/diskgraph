//! 错误后的同一原流不可继续消费；来源：PF-06 帧账本与明确终止语义。
use diskgraph_scan_worker::{Frame, FrameReader, ProtocolLimits, write_frame};
use std::io::{self, Cursor};

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 8192,
        max_stream_bytes: 32768,
        max_nodes: 10,
        max_depth: 10,
    }
}

#[test]
fn bad_json_is_charged_and_cannot_be_skipped_to_next_frame() {
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    bytes.push(b'{');
    write_frame(&mut bytes, &Frame::Cancel).unwrap();
    let mut input = Cursor::new(bytes);
    {
        let mut reader = FrameReader::new(&mut input, limits());
        assert_eq!(
            reader.read_frame().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(reader.bytes_read(), 5);
        assert!(reader.read_frame().is_err());
        assert!(reader.expect_eof().is_err());
    }
    assert_eq!(
        input.position(),
        5,
        "next frame must remain unread after first failure"
    );
}

#[test]
fn partial_body_consumes_admitted_length_and_latches_failure() {
    let mut bytes = 100_u32.to_le_bytes().to_vec();
    bytes.push(b'{');
    let mut input = Cursor::new(bytes);
    let mut reader = FrameReader::new(&mut input, limits());
    assert_eq!(
        reader.read_frame().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        reader.bytes_read(),
        104,
        "admitted bytes cannot be rolled back on short body"
    );
    assert!(
        reader.read_frame().is_err(),
        "EOF after failure must not become successful clean EOF"
    );
}
