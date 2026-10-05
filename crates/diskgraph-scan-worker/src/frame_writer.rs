use crate::{Frame, ProtocolBudgetError, ProtocolLimits, payload_buffer::PayloadBuffer};
use std::io::{self, Write};

/// 单次传输的有限写入账本；来源：PF-06，序列化期间准入，不先建立无界 JSON Vec。
pub struct FrameWriter<W> {
    output: W,
    limits: ProtocolLimits,
    bytes: u64,
    failed: bool,
}

impl<W: Write> FrameWriter<W> {
    /// 构造写入器。参数 output 与 limits 是原请求流及固定上界；返回零消费账本。
    /// 参数：output 为已有写入源，limits 为不可重置的原请求上界。
    /// 返回：零消费、未失败的有限写入器。
    pub fn new(output: W, limits: ProtocolLimits) -> Self {
        Self {
            output,
            limits,
            bytes: 0,
            failed: false,
        }
    }

    /// 有界写帧。返回累计之外的本帧长度；错误后本实例拒绝后续写入，避免部分帧续流。
    /// 参数：frame 为借用的平铺帧，self 持有累计字节账本。
    /// 返回：本帧完整写入字节数或锁存错误；原下游 IO 错误类型保持。
    pub fn write_frame(&mut self, frame: &Frame) -> io::Result<u64> {
        self.write_payload(frame)
    }

    /// 在同一有限序列化 sink 上写入专用协议载荷，保留原 Frame 兼容入口。
    /// 参数：payload 为借用的可序列化载荷；self 保持原帧和流账本。
    /// 返回：本帧完整字节数或锁存错误，不先构造无界 JSON Vec。
    pub fn write_payload<T: serde::Serialize + ?Sized>(&mut self, payload: &T) -> io::Result<u64> {
        if self.failed {
            return Err(io::Error::other("frame writer already failed"));
        }
        let result = self.write_checked(payload);
        self.failed = result.is_err();
        result
    }

    /// 借用读取下一帧正文的原剩余额度，不提前消费或刷新账本。
    /// 参数：self 为当前写入器；返回：frame/stream/u32 三项上界的最小值或已失败/无头部额度错误。
    pub(crate) fn remaining_body_bytes(&self) -> io::Result<u64> {
        if self.failed {
            return Err(io::Error::other("frame writer already failed"));
        }
        let maximum = self
            .limits
            .max_stream_bytes
            .checked_sub(self.bytes)
            .and_then(|left| left.checked_sub(4))
            .ok_or_else(|| ProtocolBudgetError::into_io("stream byte limit"))?
            .min(self.limits.max_frame_bytes)
            .min(u64::from(u32::MAX));
        Ok(maximum)
    }

    fn write_checked<T: serde::Serialize + ?Sized>(&mut self, frame: &T) -> io::Result<u64> {
        let maximum = self.remaining_body_bytes()?;
        let mut body = PayloadBuffer::new(maximum);
        let serialized = serde_json::to_writer(&mut body, frame);
        // 自定义 Serialize 可以吞掉 element 错误；发布前无条件检查同次 sink 的真实拒绝。
        if let Some(error) = body.take_failure() {
            return Err(error);
        }
        serialized.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let size = u32::try_from(body.as_slice().len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame size overflow"))?;
        let written = u64::from(size) + 4;
        let total = self
            .bytes
            .checked_add(written)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stream size overflow"))?;
        self.output.write_all(&size.to_le_bytes())?;
        self.output.write_all(body.as_slice())?;
        self.bytes = total;
        Ok(written)
    }

    /// 刷出已有帧，不重置额度；真实 IO 失败锁存，不能在部分流上继续。
    /// 参数：self 为同次有界写入器。
    /// 返回：下游 flush 的真实结果。
    pub fn flush(&mut self) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("frame writer already failed"));
        }
        let result = self.output.flush();
        self.failed = result.is_err();
        result
    }

    /// 返回已完整写出的帧字节数；部分 IO 失败不称完整成功，也不能再使用此流。
    /// 参数：self 为当前写入器。
    /// 返回：已完整写出的原始字节数，不把部分失败帧计为成功。
    pub fn bytes_written(&self) -> u64 {
        self.bytes
    }
}
