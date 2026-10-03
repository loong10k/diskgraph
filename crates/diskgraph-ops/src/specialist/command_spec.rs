//! command_spec：既有文件操作职责的原生 Rust 实现。
use std::path::PathBuf;

/// 专家工具的固定程序、结构化参数、显式环境、逐流输出上限和重试设置。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::CommandSpec`，保留既有语义。
/// A fully specified external run. Every field is structural: there is no
/// shell string anywhere in this type, so command injection has no surface.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    /// The exact program to execute. It comes from the registry's resolved
    /// path, never from user text or a project file.
    pub program: PathBuf,
    /// One argument per element, passed verbatim as one argv entry each.
    pub args: Vec<String>,
    /// The working directory, when the tool needs one.
    pub cwd: Option<PathBuf>,
    /// The whole environment the child receives: cleared first, then these.
    pub env: Vec<(String, String)>,
    pub timeout_ms: u64,
    /// Combined cap per stream; output past it is discarded and flagged.
    pub max_output_bytes: usize,
    /// How many times a failed run may be retried.
    pub retries: u32,
}
