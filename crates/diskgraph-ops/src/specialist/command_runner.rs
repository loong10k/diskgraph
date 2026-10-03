//! command_runner：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::specialist::command_spec::CommandSpec;
use crate::specialist::run_outcome::RunOutcome;

/// 专家工具执行契约，保留现有 Unix 实现与其他平台拒绝边界。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::CommandRunner`，保留既有语义。
/// Runs [`CommandSpec`]s. The ops layer never calls `std::process::Command`
/// anywhere else, so the sandbox has exactly one definition.
pub trait CommandRunner {
    /// 执行结构化专家命令。
    /// 参数：spec 指定完整执行设置。
    /// 返回：执行结果或启动、I/O、平台不支持错误。
    fn run(&self, spec: &CommandSpec) -> Result<RunOutcome, OpsError>;
}
