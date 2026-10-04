//! PF-06 原始帧字节准入；Reader 见证过大长度不得读取正文，不以 RSS 或 staging 成本代替。
use diskgraph_scan_worker::{Frame, FrameReader, ProtocolLimits, read_tree, write_frame};
use std::io::{self, Cursor, Read};

/// 只允许长度头读取的真实 Read 实现；来源：std::io 协议测试，不伪造 scanner。
struct HeaderOnly {
    header: Cursor<[u8; 4]>,
    body_reads: usize,
}

impl Read for HeaderOnly {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.header.position() >= 4 {
            self.body_reads += 1;
            return Err(io::Error::other("body must not be read"));
        }
        self.header.read(output)
    }
}

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 1024,
        max_stream_bytes: 2048,
        max_nodes: 4,
        max_depth: 3,
    }
}

#[test]
fn oversized_frame_header_is_rejected_before_body_read() {
    let mut source = HeaderOnly {
        header: Cursor::new(u32::MAX.to_le_bytes()),
        body_reads: 0,
    };
    let error = FrameReader::new(&mut source, limits())
        .read_frame()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(source.body_reads, 0);
    assert_eq!(source.header.position(), 4);
}

#[test]
fn cumulative_wire_bytes_are_not_reset_between_frames() {
    let mut one = Vec::new();
    write_frame(&mut one, &Frame::Cancel).unwrap();
    let mut data = one.clone();
    data.extend_from_slice(&one);
    let mut bound = limits();
    bound.max_stream_bytes = data.len() as u64 - 1;
    let mut reader = FrameReader::new(data.as_slice(), bound);
    assert_eq!(reader.read_frame().unwrap(), Some(Frame::Cancel));
    assert_eq!(
        reader.read_frame().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(reader.bytes_read(), one.len() as u64);
}

#[test]
fn unknown_frame_variant_and_truncated_header_or_body_are_refused() {
    let bad = br#"{"type":"invented"}"#;
    let mut data = (bad.len() as u32).to_le_bytes().to_vec();
    data.extend_from_slice(bad);
    assert_eq!(
        FrameReader::new(data.as_slice(), limits())
            .read_frame()
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        FrameReader::new(&[1_u8, 0][..], limits())
            .read_frame()
            .unwrap_err()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        FrameReader::new(&[10_u8, 0, 0, 0, 1][..], limits())
            .read_frame()
            .unwrap_err()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn eof_without_end_cannot_produce_a_tree() {
    assert_eq!(
        read_tree(&[][..], limits()).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}
