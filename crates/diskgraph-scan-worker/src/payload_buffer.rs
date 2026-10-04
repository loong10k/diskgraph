use std::io::{self, Write};

/// JSON 单帧有限 sink；来源：PF-06，追加字节前先检查本帧与原流剩余准入。
pub(crate) struct PayloadBuffer {
    bytes: Vec<u8>,
    maximum: u64,
}

impl PayloadBuffer {
    /// 参数：maximum 为当前帧与原流余额共同计算的正文最大字节。
    /// 返回：尚未分配正文的有限 Write sink。
    pub(crate) fn new(maximum: u64) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }
    /// 参数：self 为已序列化的有限 sink。
    /// 返回：借用正文切片，不再复制或扩大容量。
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

impl Write for PayloadBuffer {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let size = self
            .bytes
            .len()
            .checked_add(input.len())
            .filter(|size| *size as u64 <= self.maximum)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "frame byte limit"))?;
        self.bytes
            .try_reserve(size - self.bytes.len())
            .map_err(|_| io::Error::other("frame allocation failed"))?;
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
