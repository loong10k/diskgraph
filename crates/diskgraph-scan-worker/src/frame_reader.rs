use crate::{Frame, ProtocolLimits};
use std::io::{self, Read, Write};

/// 先读固定长度头再准入正文的有界读取器；来源：PF-06 传输账本，不共享 staging 额度。
pub struct FrameReader<R> {
    input: R,
    limits: ProtocolLimits,
    bytes: u64,
    failed: bool,
}

impl<R: Read> FrameReader<R> {
    /// 构造单流读取器。参数 input/limits 为已有管道及原请求上界；返回未重置消费的读取器。
    /// 参数：input 为已有读取源，limits 为本次传输的原始上界。
    /// 返回：零消费的单流账本。
    pub fn new(input: R, limits: ProtocolLimits) -> Self {
        Self {
            input,
            limits,
            bytes: 0,
            failed: false,
        }
    }

    /// 读取一帧。返回 None 仅代表干净 EOF，不代表 End 或实际进程已退出。
    /// 参数：self 为同一个尚未失败的帧读取器。
    /// 返回：完整帧、干净 EOF 或锁存的读取/准入/解码错误。
    pub fn read_frame(&mut self) -> io::Result<Option<Frame>> {
        self.read_payload()
    }

    /// 在原账本上读取闭合协议载荷，不新建读取器或刷新已消费字节。
    /// 参数：T 为 serde 闭合协议类型；self 保持原输入流及上界。
    /// 返回：类型化载荷、干净 EOF 或锁存错误，长度先准入再分配。
    pub fn read_payload<T: serde::de::DeserializeOwned>(&mut self) -> io::Result<Option<T>> {
        if self.failed {
            return Err(io::Error::other("frame reader already failed"));
        }
        let result = self.read_checked();
        self.failed = result.is_err();
        result
    }

    fn read_checked<T: serde::de::DeserializeOwned>(&mut self) -> io::Result<Option<T>> {
        let mut header = [0_u8; 4];
        loop {
            match self.input.read(&mut header[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        self.input.read_exact(&mut header[1..])?;
        let length = u64::from(u32::from_le_bytes(header));
        let total = self
            .bytes
            .checked_add(4)
            .and_then(|n| n.checked_add(length))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stream size overflow"))?;
        if length > self.limits.max_frame_bytes || total > self.limits.max_stream_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame byte limit",
            ));
        }
        let length = usize::try_from(length)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length overflow"))?;
        // 原始长度已准入即扣账；正文或 JSON 失败也不能回退原消费。
        self.bytes = total;
        let mut body = Vec::new();
        body.try_reserve_exact(length)
            .map_err(|_| io::Error::other("frame allocation failed"))?;
        body.resize(length, 0);
        self.input.read_exact(&mut body)?;
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// End 后只读一个字节确认 EOF；任何额外字节都拒绝，不分配或解码尾帧。
    /// 参数：self 为 End 已被上层验证的原流读取器。
    /// 返回：干净 EOF 成功；额外字节或 IO 错误拒绝并锁存。
    pub fn expect_eof(&mut self) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("frame reader already failed"));
        }
        let mut byte = [0];
        loop {
            match self.input.read(&mut byte) {
                Ok(0) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.failed = true;
                    return Err(error);
                }
                Ok(_) => {
                    self.failed = true;
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "trailing tree data",
                    ));
                }
            }
        }
    }

    /// 返回已准入并读取的原始帧字节数（含长度头），不表示 Rust/SQLite RSS。
    /// 参数：self 为当前读取器。
    /// 返回：已准入的原始头部与正文总字节，失败后也不回退。
    pub fn bytes_read(&self) -> u64 {
        self.bytes
    }
}

/// 可信测试/内部单帧工具，参数 output/frame 为已拥有数据；返回真实写入字节数。
/// 不含调用者累计配额，产品必须使用显式 limits 的 FrameWriter/TreeWriter。
/// 参数：output 为可信测试或内部写入源，frame 为已拥有的单条帧。
/// 返回：实际头部与正文长度或 IO/编码错误；不提供累计请求配额。
pub fn write_frame(output: &mut impl Write, frame: &Frame) -> io::Result<u64> {
    let body = serde_json::to_vec(frame).map_err(io::Error::other)?;
    let length = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(&body)?;
    Ok(u64::from(length) + 4)
}
