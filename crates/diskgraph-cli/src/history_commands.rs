//! CLI history_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::snapshot_reply;
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
    deadline: std::time::Instant,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Growth {
            scope,
            before,
            after,
            path,
        } => {
            let scope = ScopeId::new(scope.clone()).map_err(|_| BusinessError::InvalidArgument)?;
            engine.authorize_revision_until(
                Some(&scope),
                before,
                principal,
                authorizer,
                deadline,
            )?;
            engine.authorize_revision_until(
                Some(&scope),
                after,
                principal,
                authorizer,
                deadline,
            )?;
            let growth = engine.growth_between_until(
                before,
                after,
                std::path::Path::new(path),
                snapshot_reply::budget(),
                principal,
                authorizer,
                deadline,
            )?;
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[before, after],
                serde_json::json!({
                    "delta_bytes": growth.as_ref().map(|g| g.delta_bytes.to_string()),
                    "comparable": growth.is_some(),
                    "path": if path.is_empty() { ".".to_owned() } else { path.clone() },
                }),
                deadline,
            )?);
            Ok(())
        }
        Command::Changes {
            scope,
            before,
            after,
        } => {
            let scope = ScopeId::new(scope.clone()).map_err(|_| BusinessError::InvalidArgument)?;
            engine.authorize_revision_until(
                Some(&scope),
                before,
                principal,
                authorizer,
                deadline,
            )?;
            engine.authorize_revision_until(
                Some(&scope),
                after,
                principal,
                authorizer,
                deadline,
            )?;
            let data = engine.revision_changes_until(
                before,
                after,
                snapshot_reply::budget(),
                principal,
                authorizer,
                deadline,
            )?;
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[before, after],
                data,
                deadline,
            )?);
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
