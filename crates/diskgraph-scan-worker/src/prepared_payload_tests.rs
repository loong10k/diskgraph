//! 原有限准备与同账本提交边界；来源：真实 serde 调用及 std::fs 只读句柄错误。
use crate::{Frame, FrameWriter, ProtocolBudgetError, ProtocolLimits};
use std::cell::Cell;
use std::io::{self, Write};

/// 先接受真实帧头、正文再使用只读 OS 句柄的输出端；来源：std::io::Write。
struct HeaderThenReadOnly {
    read_only: std::fs::File,
    header: Vec<u8>,
    calls: usize,
}

impl Write for HeaderThenReadOnly {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            self.header.extend_from_slice(input);
            Ok(input.len())
        } else {
            self.read_only.write(input)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.read_only.flush()
    }
}

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: 1024,
        max_stream_bytes: 4096,
        max_nodes: 8,
        max_depth: 4,
    }
}

#[test]
fn data_preparation_serializes_once_and_commit_uses_same_bytes() {
    /// 内部借用序列化见证；字段只记录实际调用次数，无复制后的替代载荷。
    struct Once<'a>(&'a Cell<usize>);
    impl serde::Serialize for Once<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            serializer.serialize_str("original")
        }
    }
    let calls = Cell::new(0);
    let mut output = Vec::new();
    {
        let mut writer = FrameWriter::new(&mut output, limits());
        let prepared = writer.prepare_reserving(&Once(&calls), 128).unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(prepared.as_slice(), b"\"original\"");
        assert_eq!(writer.bytes_written(), 0);
        assert_eq!(writer.write_prepared(prepared).unwrap(), 14);
        assert_eq!(calls.get(), 1);
        assert_eq!(writer.bytes_written(), 14);
    }
    assert_eq!(&output[..4], &10_u32.to_le_bytes());
    assert_eq!(&output[4..], b"\"original\"");
}

#[test]
fn quota_preflight_preserves_terminal_space_in_original_ledger() {
    let mut bound = limits();
    bound.max_stream_bytes = 32;
    let mut output = Vec::new();
    {
        let mut writer = FrameWriter::new(&mut output, bound);
        let first = writer.prepare_reserving(&"x".repeat(64), 21).err().unwrap();
        assert_eq!(first.kind(), io::ErrorKind::InvalidData);
        assert!(
            first
                .get_ref()
                .is_some_and(|cause| cause.is::<ProtocolBudgetError>())
        );
        assert_eq!(writer.bytes_written(), 0);
        // 实际终态不再扣第二份 reserve；仍受原 32 字节总额限制。
        assert_eq!(writer.write_frame(&Frame::Cancel).unwrap(), 21);
        assert_eq!(writer.bytes_written(), 21);
        assert!(writer.write_frame(&Frame::Cancel).is_err());
        assert_eq!(writer.bytes_written(), 21);
    }
    assert_eq!(output.len(), 21);
}

#[test]
fn ordinary_prepare_error_latches_without_using_budget_text_as_classification() {
    /// 内部真实 serde 错误来源；不是 sink 的额度拒绝。
    struct Ordinary;
    impl serde::Serialize for Ordinary {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("frame byte limit"))
        }
    }
    let mut output = Vec::new();
    {
        let mut writer = FrameWriter::new(&mut output, limits());
        let first = writer.prepare_reserving(&Ordinary, 128).err().unwrap();
        assert_eq!(first.kind(), io::ErrorKind::InvalidData);
        assert_eq!(first.to_string(), "frame byte limit");
        assert!(
            !first
                .get_ref()
                .is_some_and(|cause| cause.is::<ProtocolBudgetError>())
        );
        assert_eq!(writer.bytes_written(), 0);
        assert_eq!(
            writer.write_frame(&Frame::Cancel).unwrap_err().to_string(),
            "frame writer already failed"
        );
    }
    assert!(output.is_empty());
}

#[test]
fn actual_body_io_error_after_header_preserves_errno_and_blocks_terminal_append() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut reference = std::fs::File::open(file.path()).unwrap();
    let expected = reference.write_all(b"actual error").unwrap_err();
    let mut output = HeaderThenReadOnly {
        read_only: std::fs::File::open(file.path()).unwrap(),
        header: Vec::new(),
        calls: 0,
    };
    {
        let mut writer = FrameWriter::new(&mut output, limits());
        let prepared = writer.prepare_reserving(&"data", 128).unwrap();
        let first = writer.write_prepared(prepared).unwrap_err();
        assert_eq!(first.kind(), expected.kind());
        assert_eq!(first.raw_os_error(), expected.raw_os_error());
        assert_eq!(first.to_string(), expected.to_string());
        assert_eq!(writer.bytes_written(), 0);
        assert_eq!(
            writer.write_frame(&Frame::Cancel).unwrap_err().to_string(),
            "frame writer already failed"
        );
    }
    assert_eq!(output.header, 6_u32.to_le_bytes());
    assert_eq!(
        output.calls, 2,
        "failed writer must not retry or append terminal"
    );
}
