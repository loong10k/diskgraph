//! CLI node_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::authorization::require_metadata;
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, QueryBudget, ScopeId};
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
        Command::Node { scope } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let root = engine.revision_root_node(&revision)?;
            let coverage = engine.revision_snapshot(&revision)?.coverage;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "node": root,
                    "coverage": coverage,
                })),
            ));
            Ok(())
        }
        Command::Children {
            scope,
            parent_id,
            limit,
            offset,
            min_bytes,
            unknown_only,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            if *unknown_only && min_bytes.is_some() {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            let (items, next_offset, unknown_count) = if *unknown_only {
                let (items, next_offset) = engine
                    .revision_unknown_children_page(&revision, *parent_id, *offset, *limit)?;
                (items, next_offset, 0)
            } else {
                engine.revision_children_page(&revision, *parent_id, *min_bytes, *offset, *limit)?
            };
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "size_kind": "allocated",
                    "items": items,
                    "next_offset": next_offset,
                    // Nodes whose size could not be reported are counted, not
                    // silently dropped or treated as zero.
                    "unknown_size_count": unknown_count,
                })),
            ));
            Ok(())
        }
        Command::Top {
            scope,
            parent_id,
            limit,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let (items, more) = engine.revision_top(&revision, *parent_id, *limit)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "size_kind": "allocated",
                    "items": items,
                    "truncated": more.then_some("node_limit"),
                })),
            ));
            Ok(())
        }
        Command::Explore {
            scope,
            node_id,
            max_depth,
            max_nodes,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let budget = QueryBudget {
                max_depth: *max_depth,
                max_nodes: *max_nodes,
                ..QueryBudget::default()
            };
            let page_limit = budget.max_nodes.min(100);
            let (node, children, more) =
                engine.revision_layer_page(&revision, *node_id, 0, page_limit)?;
            let coverage = engine.revision_snapshot(&revision)?.coverage;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "node": node,
                    "children": children,
                    "coverage": coverage,
                    "truncated": (more || page_limit == 0).then_some("node_limit"),
                })),
            ));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
