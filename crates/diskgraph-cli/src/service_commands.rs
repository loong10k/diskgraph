//! CLI service_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use diskgraph_core::BusinessError;
use diskgraph_engine::{Engine, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(engine: &Engine, cli: &Cli, out: &mut Vec<String>) -> Result<(), EngineError> {
    match &cli.command {
        Command::Doctor => {
            let profile = diskgraph_mcp::protocol::ToolProfile::ReadFull;
            let report = diskgraph_mcp::doctor::diagnose(engine, profile);
            // A degraded report is data, not a tool failure.
            out.push(envelope_line(engine, Ok(report.to_json(engine, profile))));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
