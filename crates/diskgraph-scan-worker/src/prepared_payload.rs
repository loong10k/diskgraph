use crate::payload_buffer::PayloadBuffer;
use std::io;

/// 单次有限序列化的拥有正文；来源：PF-06，准备不触碰输出，提交不复制或再次序列化。
pub(crate) struct PreparedPayload {
    body: PayloadBuffer,
}

impl PreparedPayload {
    /// 参数：payload 为借用数据，maximum 为原帧及扣除终态预留后的原流正文上限。
    /// 返回：唯一有限正文，或真实 sink 首错；Serialize 吞错也不能绕过发布门禁。
    pub(crate) fn new<T: serde::Serialize + ?Sized>(payload: &T, maximum: u64) -> io::Result<Self> {
        let mut body = PayloadBuffer::new(maximum);
        let serialized = serde_json::to_writer(&mut body, payload);
        if let Some(error) = body.take_failure() {
            return Err(error);
        }
        serialized.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Self { body })
    }

    /// 参数：self 为已经通过完整序列化及首错检查的正文。
    /// 返回：原缓冲的借用字节；不转移成可被调用者改写的 Vec。
    pub(crate) fn as_slice(&self) -> &[u8] {
        self.body.as_slice()
    }
}
