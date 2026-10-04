/// 单次传输的显式上界；来源：PF-06 原生 Rust helper 协议，不是 staging 额度。
#[derive(Clone, Copy, Debug)]
pub struct ProtocolLimits {
    pub max_frame_bytes: u64,
    pub max_stream_bytes: u64,
    pub max_nodes: u64,
    pub max_depth: u64,
}
