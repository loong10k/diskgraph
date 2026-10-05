use crate::ProtocolLimits;

/// 执行 Request/Cancel 的固定输入上限，独立于扫描响应额度。
/// 来源：PF-06 helper 接收端原有 1 MiB 单帧、2 MiB 累计输入合同。
pub struct WorkerInputLimits;

impl WorkerInputLimits {
    /// 参数：无，客户端响应额度不能扩展或缩小此输入上限。
    /// 返回：父端序列化与 helper 接收端共享的固定协议额度。
    pub const fn protocol_limits() -> ProtocolLimits {
        ProtocolLimits {
            max_frame_bytes: 1024 * 1024,
            max_stream_bytes: 2 * 1024 * 1024,
            max_nodes: 1,
            max_depth: 0,
        }
    }
}
