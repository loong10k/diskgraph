use crate::ProtocolLimits;
use std::io;

/// 无 I/O 的 4B/正文碎片拼接器；来源：FrameReader 原长度准入规则，每次只交付一个完整正文。
pub(crate) struct IncrementalFrameDecoder {
    limits: ProtocolLimits,
    header: [u8; 4],
    header_used: usize,
    body_length: usize,
    body: Vec<u8>,
    bytes: u64,
}

impl IncrementalFrameDecoder {
    /// 参数：limits 为父端原始单流字节预算。
    /// 返回：未拥有正文的拼接状态；不建立新时钟或读管道。
    pub(crate) fn new(limits: ProtocolLimits) -> Self {
        Self {
            limits,
            header: [0; 4],
            header_used: 0,
            body_length: 0,
            body: Vec::new(),
            bytes: 0,
        }
    }

    /// 参数：input 为现非阻塞管道一次读取的借用片段。
    /// 返回：实际消耗数及最多一个完整正文；长度在拥有正文前准入并永久扣账。
    pub(crate) fn push(&mut self, input: &[u8]) -> io::Result<(usize, Option<Vec<u8>>)> {
        let mut consumed = 0;
        if self.header_used < 4 {
            let take = (4 - self.header_used).min(input.len());
            self.header[self.header_used..self.header_used + take].copy_from_slice(&input[..take]);
            self.header_used += take;
            consumed += take;
            if self.header_used < 4 {
                return Ok((consumed, None));
            }
            let length = u64::from(u32::from_le_bytes(self.header));
            let total = self
                .bytes
                .checked_add(4)
                .and_then(|n| n.checked_add(length))
                .ok_or_else(|| invalid("stream size overflow"))?;
            if length > self.limits.max_frame_bytes || total > self.limits.max_stream_bytes {
                return Err(invalid("frame byte limit"));
            }
            self.body_length =
                usize::try_from(length).map_err(|_| invalid("frame length overflow"))?;
            // 准入失败不扣账；已准入正文即使短读、分配或解码失败也不退款。
            self.bytes = total;
            self.body
                .try_reserve_exact(self.body_length)
                .map_err(|_| io::Error::other("frame allocation failed"))?;
        }
        let take = (self.body_length - self.body.len()).min(input.len() - consumed);
        self.body
            .extend_from_slice(&input[consumed..consumed + take]);
        consumed += take;
        if self.body.len() == self.body_length {
            self.header_used = 0;
            self.body_length = 0;
            return Ok((consumed, Some(std::mem::take(&mut self.body))));
        }
        Ok((consumed, None))
    }

    /// 参数：self 为原拼接器。
    /// 返回：已声明准入的头与正文总字节，包含未到齐正文。
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// 参数：self 为管道已报告实际 EOF 的拼接器。
    /// 返回：仅完整帧边界成功；部分头部或正文拒绝。
    pub(crate) fn finish_eof(&self) -> io::Result<()> {
        if self.header_used != 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "partial execution frame",
            ));
        }
        Ok(())
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
