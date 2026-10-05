use crate::ProtocolLimits;
use serde::{Deserialize, Serialize};
use std::io;

/// 执行协议 v2 的显式传输额度；来源：PF-06，与旧 ScanRequest 及 staging 账本分离。
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerLimits {
    max_frame_bytes: u64,
    max_stream_bytes: u64,
    max_nodes: u64,
    max_depth: u64,
}

impl WorkerLimits {
    /// 从父端原账本构造明确传输额度，不重设已消费值或业务时钟。
    /// 参数：limits 为本次执行传输的四项上界。
    /// 返回：通过表示范围校验的 v2 额度记录；非法上界明确拒绝。
    pub fn from_protocol(limits: ProtocolLimits) -> io::Result<Self> {
        let value = Self {
            max_frame_bytes: limits.max_frame_bytes,
            max_stream_bytes: limits.max_stream_bytes,
            max_nodes: limits.max_nodes,
            max_depth: limits.max_depth,
        };
        value.checked()?;
        Ok(value)
    }

    /// 参数：self 为闭合执行请求中携带的原始额度。
    /// 返回：可表示且非空的单流额度；不替换调用者节点或深度约束。
    pub fn checked(&self) -> io::Result<ProtocolLimits> {
        if self.max_frame_bytes == 0
            || self.max_frame_bytes > u64::from(u32::MAX)
            || self.max_stream_bytes < 4
            || self.max_nodes == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid transport limits",
            ));
        }
        Ok(ProtocolLimits {
            max_frame_bytes: self.max_frame_bytes,
            max_stream_bytes: self.max_stream_bytes,
            max_nodes: self.max_nodes,
            max_depth: self.max_depth,
        })
    }
}
