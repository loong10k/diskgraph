use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// 一次实时采样的共享资源配置，不随子命令重置。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProbeLimits。
/// Unix 宿主不得自动/外部回收本次 child；Windows 需创建时 Job 属性支持。
/// 安全清理及内核 I/O 可能超出协作期限，预算不构成严格 RSS 或墙钟保证。
#[derive(Clone, Debug)]
pub struct ProbeLimits {
    /// 整次采样的协作期限，安全回收和内核等待可能超过该期限。
    pub timeout: Duration,
    /// 全部子命令 stdout/stderr 累计允许保留的字节，最多 64 MiB。
    pub max_output_bytes: usize,
    /// 调用方设置为 true 后停止后续采样并回收本次执行域。
    pub cancel: Arc<AtomicBool>,
}

impl Default for ProbeLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            max_output_bytes: 1 << 20,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
