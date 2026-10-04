//! CLI search_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::output::envelope_line;
use diskgraph_core::{
    Authorizer, BusinessError, CursorContext, PagingCursor, PrincipalId, ScopeId,
};
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
        Command::Search {
            scope,
            pattern,
            offset,
            limit,
            cursor,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            // A cursor is only honored when it was issued for this same
            // principal, scope, revision, pattern, and sort order (Q-02).
            let pattern_binding = format!("pattern:{pattern}");
            let context = CursorContext {
                principal_binding: &cli.principal,
                scope_id: scope_id.as_str(),
                revision_id: &revision,
                filter_binding: &pattern_binding,
                sort_binding: "name_asc,id_asc,keyset_v2",
                policy_version: authorizer.policy_version(),
            };
            let cursor = cursor
                .as_ref()
                .map(|encoded| diskgraph_core::SearchCursor::decode(encoded, &context))
                .transpose()
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision(Some(&scope_id), &revision, principal, authorizer)?;
            let reader = engine.revision_reader()?;
            let snapshot = reader.revision(&revision)?.snapshot_id;
            let after = cursor
                .as_ref()
                .map(|cursor| (cursor.last_name.as_str(), cursor.last_id));
            let (items, more) =
                reader.search_page(&snapshot, pattern, after, *offset, (*limit).clamp(1, 100))?;
            let consumed = cursor
                .as_ref()
                .map_or(*offset, |cursor| cursor.binding.offset)
                .saturating_add(items.len() as u64);
            let next_cursor = items.last().filter(|_| more).map(|last| {
                diskgraph_core::SearchCursor {
                    version: 2,
                    binding: PagingCursor::issue(
                        &context,
                        pattern_binding.clone(),
                        context.sort_binding,
                        consumed,
                    ),
                    last_name: last.name.clone(),
                    last_id: last.id,
                }
                .encode()
            });
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "items": items,
                    "next_cursor": next_cursor,
                })),
            ));
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
