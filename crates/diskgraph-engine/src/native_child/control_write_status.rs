/// 单个自有控制数据块的原生写状态；来源：Unix send 与 Windows overlapped I/O。
/// 写完成仅表示内核接收了字节，不证明 worker 已读取或整个协议帧已发送。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControlWriteStatus {
    /// 当前数据块实际完成的字节数，调用方据此推进原帧游标。
    Written(usize),
    /// 本数据块仍由通道持有，只能轮询，不得覆盖或重新发送。
    Pending,
    /// 输入已关闭，且没有未完成 I/O 借用缓冲区。
    Closed,
}

impl ControlWriteStatus {
    /// 每次拥有的控制块最多 4096 字节，不随外部帧长度扩大缓冲区。
    pub(crate) const MAX_CHUNK_BYTES: usize = 4096;
}
