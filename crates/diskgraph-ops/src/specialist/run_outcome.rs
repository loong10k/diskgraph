//! run_outcome：既有文件操作职责的原生 Rust 实现。

/// 专家进程的退出、逐流输出、超时和截断结果；不能单凭退出码确认操作效果。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::RunOutcome`，保留既有语义。
/// What a run produced. `truncated` and `timed_out` exist so a caller can
/// refuse to interpret output it did not fully see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    /// The child's exit code, or -1 when it was killed or never started.
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub truncated: bool,
}
