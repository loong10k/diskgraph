//! CLI grant_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use diskgraph_core::{BusinessError, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    out: &mut Vec<String>,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Grant {
            scope,
            content_read,
            revoke_content_read,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.scope(&scope_id)?;
            let allow = *content_read || !*revoke_content_read;
            engine.set_content_read(&scope_id, principal, allow)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "scope_id": scope_id.as_str(),
                    "content_read": if allow { "granted" } else { "revoked" },
                })),
            ));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
