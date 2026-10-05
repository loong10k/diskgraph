use std::io::{self, Write};

/// 已准入输入帧的有限、可失败分配存储，不先复制整个 WorkerRequest。
/// 来源：PF-06 父驱动；正文序列化准入仍由原 FrameWriter 执行。
pub(super) struct ScanWorkerInputBuffer {
    bytes: Vec<u8>,
    maximum: u64,
}

impl ScanWorkerInputBuffer {
    /// 参数：maximum 为原输入流剩余额度，包含本帧四字节头部。
    /// 返回：未分配正文的有限下游 sink。
    pub(super) fn new(maximum: u64) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }

    /// 参数：self 为完整序列化后的原帧存储。
    /// 返回：借用实际字节，不克隆路径、选项或 JSON。
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 参数：maximum 为扣除先前已序列化输入后的原余额。
    /// 返回：复用有限容量以承接下一帧，不恢复原输入账本。
    pub(super) fn reset(&mut self, maximum: u64) {
        self.bytes.clear();
        self.maximum = maximum;
    }
}

impl Write for ScanWorkerInputBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length as u64 <= self.maximum)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "input byte limit"))?;
        self.bytes
            .try_reserve(length - self.bytes.len())
            .map_err(|_| io::Error::other("input frame allocation failed"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // 此 sink 只拥有内存；真正控制写入完成由 Child 报告。
        Ok(())
    }
}
