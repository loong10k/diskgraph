//! 真实下游在接收长度头后失败；来源：std::io，原始错误必须保留且同流不能续写。
use diskgraph_scan_worker::{Frame, FrameWriter, ProtocolLimits};
use std::cell::Cell;
use std::io::{self, Write};

/// 接收完整四字节头后，在正文 write 返回确定 BrokenPipe 的真实 Write 实现。
struct HeaderThenFail<'a> {
    header_bytes: usize,
    calls: &'a Cell<usize>,
}

impl Write for HeaderThenFail<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls.set(self.calls.get() + 1);
        if self.header_bytes < 4 {
            let accepted = bytes.len().min(4 - self.header_bytes);
            self.header_bytes += accepted;
            return Ok(accepted);
        }
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "controlled body failure",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn body_io_error_preserves_kind_and_latches_after_actual_header_write() {
    let calls = Cell::new(0);
    let mut output = HeaderThenFail {
        header_bytes: 0,
        calls: &calls,
    };
    let limits = ProtocolLimits {
        max_frame_bytes: 1024,
        max_stream_bytes: 4096,
        max_nodes: 10,
        max_depth: 10,
    };
    {
        let mut writer = FrameWriter::new(&mut output, limits);
        let error = writer.write_frame(&Frame::Cancel).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(error.to_string(), "controlled body failure");
        assert_eq!(
            calls.get(),
            2,
            "header and body writes must actually be attempted"
        );
        assert_eq!(writer.bytes_written(), 0, "partial frame is not completed");
        assert!(writer.write_frame(&Frame::Cancel).is_err());
        assert_eq!(
            calls.get(),
            2,
            "failed stream cannot receive another header"
        );
    }
    assert_eq!(output.header_bytes, 4);
}
