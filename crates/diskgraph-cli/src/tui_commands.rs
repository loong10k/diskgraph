//! CLI tui_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::authorization::require_metadata;
use crate::cli::Cli;
use crate::command::Command;
use crate::tui;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineError};

/// 执行本组真实业务命令，保持原授权、预算、响应与副作用顺序。
/// 参数：本请求的解析参数及对应 Engine 依赖。返回：执行成功或原业务错误。
pub(crate) fn run(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Tui {
            scope,
            revision,
            anonymize,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let revision = match revision {
                Some(revision) => revision.clone(),
                None => engine
                    .latest_revision(&scope_id)?
                    .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
            };
            engine.authorize_revision(Some(&scope_id), &revision, principal, authorizer)?;
            tui::run(engine, &revision, principal, authorizer, *anonymize)?;
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
