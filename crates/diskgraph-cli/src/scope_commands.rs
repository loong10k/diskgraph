//! CLI scope_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use crate::scope_action::ScopeAction;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, ScopeId};
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
        Command::Scope { action } => match action {
            ScopeAction::Add { root } => {
                let scope_id = engine.register_scope(root, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "scope_id": scope_id.as_str() })),
                ));
                Ok(())
            }
            ScopeAction::List => {
                let scopes = engine.list_scopes(principal, authorizer)?;
                let ids: Vec<&str> = scopes.iter().map(|scope| scope.scope_id.as_str()).collect();
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "scopes": ids })),
                ));
                Ok(())
            }
            ScopeAction::Show { scope } => {
                let scope_id = ScopeId::new(scope.clone())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                let record = engine.scope(&scope_id)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({
                        "scope_id": record.scope_id.as_str(),
                        "root_display": record.root.display(),
                        "revoked": record.revoked,
                    })),
                ));
                Ok(())
            }
            ScopeAction::Remove { scope } => {
                let scope_id = ScopeId::new(scope.clone())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                engine.revoke_scope(&scope_id, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "revoked": scope_id.as_str() })),
                ));
                Ok(())
            }
        },
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
