//! CLI snapshot_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use crate::snapshot_action::SnapshotAction;
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
        Command::Snapshots {
            scope,
            limit,
            offset,
            action,
        } => {
            if let Some(SnapshotAction::Prune {
                scope,
                keep_last,
                apply,
            }) = action
            {
                let scope_id = ScopeId::new(scope.clone())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                let candidates =
                    engine.prune_snapshots(&scope_id, *keep_last, *apply, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({"applied":apply, "candidates":candidates})),
                ));
                return Ok(());
            }
            let scope_id = ScopeId::new(
                scope
                    .clone()
                    .ok_or(EngineError::Business(BusinessError::InvalidArgument))?,
            )
            .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            let snapshots =
                engine.list_snapshots(&scope_id, principal, authorizer, *limit, *offset)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "snapshots": snapshots.iter().map(|snapshot| serde_json::json!({
                        "snapshot_id": snapshot.id,
                        "captured_at_unix_ms": snapshot.captured_at_unix_ms,
                        "complete": snapshot.coverage.complete,
                    })).collect::<Vec<_>>(),
                })),
            ));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
