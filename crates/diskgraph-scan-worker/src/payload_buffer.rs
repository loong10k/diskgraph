use crate::ProtocolBudgetError;
use std::io::{self, Write};

/// JSON 单帧有限 sink；来源：PF-06，追加字节前先检查本帧与原流剩余准入。
pub(crate) struct PayloadBuffer {
    bytes: Vec<u8>,
    maximum: u64,
    failure: Option<io::Error>,
}

impl PayloadBuffer {
    /// 参数：maximum 为当前帧与原流余额共同计算的正文最大字节。
    /// 返回：尚未分配正文的有限 Write sink。
    pub(crate) fn new(maximum: u64) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
            failure: None,
        }
    }
    /// 参数：self 为已序列化的有限 sink。
    /// 返回：借用正文切片，不再复制或扩大容量。
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// 参数：self 为本次序列化独占的有限 sink。
    /// 返回：取出首个真实 sink 错误，保留额度与分配失败的原始类型，不识别文本。
    pub(crate) fn take_failure(&mut self) -> Option<io::Error> {
        self.failure.take()
    }
}

impl Write for PayloadBuffer {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        // 首次追加失败后停止接收，避免自定义序列化器吞错后继续拼接残片。
        if self.failure.is_some() {
            return Err(io::Error::other("payload buffer already failed"));
        }
        let result = self
            .bytes
            .len()
            .checked_add(input.len())
            .filter(|size| *size as u64 <= self.maximum)
            .ok_or_else(|| ProtocolBudgetError::into_io("frame byte limit"))
            .and_then(|size| {
                self.bytes
                    .try_reserve(size - self.bytes.len())
                    .map_err(|_| io::Error::other("frame allocation failed"))
            });
        if let Err(error) = result {
            // 保存原错误供发布门禁取得；serde 的包装或后续错误不能替换首次失败。
            let returned = if error
                .get_ref()
                .is_some_and(|cause| cause.is::<ProtocolBudgetError>())
            {
                ProtocolBudgetError::into_io("frame byte limit")
            } else {
                io::Error::other("frame allocation failed")
            };
            self.failure = Some(error);
            return Err(returned);
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
