//! CLI tree_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::authorization::require_metadata;
use crate::cli::Cli;
use crate::command::Command;
use crate::html;
use crate::output::envelope_line;
#[cfg(test)]
use crate::query_terminal_tests;
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
        Command::Tree {
            scope,
            revision,
            depth,
            min_bytes,
            html,
            anonymize,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let revision = match revision {
                Some(revision) => revision.clone(),
                None => engine
                    .latest_revision_until(&scope_id, deadline)?
                    .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
            };
            // The narrow read path: no full-graph materialization. A pre-v4
            // snapshot falls back inside the engine and renders identically.
            let mut view = engine.tree_view_until(
                &scope_id,
                &revision,
                principal,
                authorizer,
                *depth,
                *min_bytes,
                snapshot_reply::budget(),
                deadline,
            )?;
            if *anonymize {
                html::anonymize_tree(&mut view.root, "home");
            }
            if let Some(destination) = html {
                let scope_record = engine.scope(&scope_id)?;
                let root = scope_record
                    .root
                    .raw_bytes()
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_else(|| scope.clone());
                // A shared report must not disclose the user name in the root
                // path; the anonymized root label replaces it.
                let root_label = if *anonymize {
                    "home".to_owned()
                } else {
                    root.clone()
                };
                let truncated = html::tree_is_truncated(&view.root);
                let page = html::render_page(&view.root, &root_label, &revision, truncated, *depth);
                #[cfg(test)]
                query_terminal_tests::before_reply();
                if !snapshot_reply::finalize(engine, principal, authorizer, &[&revision], deadline)?
                {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                std::fs::write(destination, page)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({
                        "revision_id": revision,
                        "rendered_depth": depth,
                        "written": destination.display().to_string(),
                    })),
                ));
                return Ok(());
            }
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[&revision],
                serde_json::json!({
                    "revision_id": revision,
                    "rendered_depth": depth,
                    "tree": view.root,
                }),
                deadline,
            )?);
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
