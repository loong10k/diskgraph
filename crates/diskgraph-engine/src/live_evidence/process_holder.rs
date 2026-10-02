//! 记录精确采样路径可见的进程 PID 与命令名称。

/// 记录精确采样路径可见的进程 PID 与命令名称。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProcessHolder。
/// A process that holds one of the sampled paths open.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProcessHolder {
    pub pid: u32,
    pub command: String,
}
