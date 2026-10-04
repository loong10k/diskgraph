//! CLI disabled_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::unsupported_command::unsupported;
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(cli: &Cli) -> Result<(), EngineError> {
    match &cli.command {
        Command::Duplicates => Err(unsupported("C17", "P7")),
        Command::Read { scope: _ } => Err(unsupported("C18", "P7")),
        Command::Move { scope: _ } => Err(unsupported("C19", "P5")),
        Command::Copy { scope: _ } => Err(unsupported("C20", "P5")),
        Command::Trash { scope: _ } => Err(unsupported("C21", "P5")),
        Command::Restore { scope: _ } => Err(unsupported("C22", "P5")),
        Command::Purge { scope: _ } => Err(unsupported("C23", "P6")),
        Command::Plan { action: _ } => Err(unsupported("C24", "P5")),
        Command::Apply { scope: _ } => Err(unsupported("C25", "P5")),
        Command::Operations { action: _ } => Err(unsupported("C26", "P5")),
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
