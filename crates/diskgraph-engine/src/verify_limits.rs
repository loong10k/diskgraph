use std::sync::{Arc, atomic::AtomicBool};

/// 一次内容比较的共享资源上限，失败路径和双侧读取共同消耗字节与期限。
/// 来源：原生 Rust diskgraph-engine::VerifyLimits。
pub struct VerifyLimits {
    pub max_total_bytes: u64,
    pub max_duration_ms: u64,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for VerifyLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 512 << 20,
            max_duration_ms: 30_000,
            cancel: None,
        }
    }
}
