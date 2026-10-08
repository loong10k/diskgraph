//! CLI relation_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::cli::Cli;
use crate::command::Command;
use crate::relation_reply;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, QueryBudget, Relation, ScopeId};
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
        Command::Explain {
            scope,
            revision,
            entity,
            limit,
            after_edge,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision_until(
                Some(&scope_id),
                revision,
                principal,
                authorizer,
                deadline,
            )?;
            let server = engine.server_id_until(deadline)?;
            let mut encoded = None;
            engine.explain_bounded_with_finish_until(
                revision,
                entity,
                after_edge.as_deref(),
                *limit,
                QueryBudget::default(),
                principal,
                authorizer,
                deadline,
                |data, expired| {
                    encoded = Some(relation_reply::encode_within_request(
                        server.clone(),
                        data,
                        expired,
                    )?);
                    Ok(())
                },
            )?;
            out.push(encoded.ok_or(BusinessError::BudgetExceeded)?);
            Ok(())
        }
        Command::Related {
            scope,
            revision,
            entity,
            relation,
            outgoing,
            limit,
            after_edge,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision_until(
                Some(&scope_id),
                revision,
                principal,
                authorizer,
                deadline,
            )?;
            let relation = match relation {
                Some(name) => Some(
                    Relation::parse(name)
                        .ok_or(EngineError::Business(BusinessError::InvalidArgument))?,
                ),
                None => None,
            };
            let server = engine.server_id_until(deadline)?;
            let mut encoded = None;
            engine.related_bounded_with_finish_until(
                revision,
                entity,
                relation,
                *outgoing,
                after_edge.as_deref(),
                *limit,
                QueryBudget::default(),
                principal,
                authorizer,
                deadline,
                |data, expired| {
                    encoded = Some(relation_reply::encode_within_request(
                        server.clone(),
                        data,
                        expired,
                    )?);
                    Ok(())
                },
            )?;
            out.push(encoded.ok_or(BusinessError::BudgetExceeded)?);
            Ok(())
        }
        Command::Impact {
            scope,
            revision,
            entity,
            max_depth,
            max_edges,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision_until(
                Some(&scope_id),
                revision,
                principal,
                authorizer,
                deadline,
            )?;
            let budget = QueryBudget {
                max_depth: *max_depth,
                max_edges: *max_edges,
                ..QueryBudget::default()
            };
            let server = engine.server_id_until(deadline)?;
            let mut encoded = None;
            engine.revision_impact_with_finish_until(
                revision,
                entity,
                budget,
                principal,
                authorizer,
                deadline,
                |answer, expired| {
                    let data = serde_json::json!({
                        "revision_id": revision,
                        "entity": entity,
                        "entries": answer.entries.iter().map(|entry| serde_json::json!({
                            "entity_id": entry.entity_id,
                            "relation": entry.relation.wire_name(),
                            "depth": entry.depth,
                        })).collect::<Vec<_>>(),
                        "grants_execution": false,
                        "complete": answer.truncated.is_none(),
                        "truncated": answer.truncated.map(|reason|reason.wire_name()),
                    });
                    encoded = Some(relation_reply::encode_within_request(
                        server.clone(),
                        &data,
                        expired,
                    )?);
                    Ok(())
                },
            )?;
            // 编码只是准备文本；原请求终检成功后才交给输出适配器。
            out.push(encoded.ok_or(BusinessError::BudgetExceeded)?);
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
