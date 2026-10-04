//! CLI policy_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use crate::policy_command::PolicyCommand;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId};
use diskgraph_engine::{Engine, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Policy(action) => match action {
            PolicyCommand::Publish { version } => {
                engine.publish_policy_version(*version, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({"published_version": version})),
                ));
                Ok(())
            }
            PolicyCommand::Revoke => {
                engine.revoke_policy(principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({"revoked": true})),
                ));
                Ok(())
            }
        },
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
