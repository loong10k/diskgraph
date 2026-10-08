//! CLI candidate_commands 的实际命令分发。来源：原生 main::dispatch。
use crate::authorization::require_metadata;
use crate::cli::Cli;
use crate::command::Command;
use crate::relation_reply;
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
    deadline: std::time::Instant,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Candidates {
            scope,
            target_bytes,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // 范围查询沿用原期限；不存在的范围仍先返回 not_found。
            engine.scope_until(&scope_id, deadline)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision_until(&scope_id, deadline)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let server = engine.server_id_until(deadline)?;
            let mut encoded = None;
            engine.review_candidates_with_finish_until(
                &revision,
                *target_bytes,
                QueryBudget::default(),
                principal,
                authorizer,
                deadline,
                |answer, expired| {
                    let candidates: Vec<_> = answer.candidates.iter().map(|(node, evidence)| {
                        serde_json::json!({ "node": node, "evidence": evidence })
                    }).collect();
                    let data = serde_json::json!({
                        "candidates": candidates,
                        "review_only": true,
                        "coverage_complete": answer.coverage_complete,
                        "coverage_observed": answer.coverage_observed,
                        "complete": answer.complete,
                        "truncated": answer.truncated.map(|reason| reason.wire_name()),
                        "selected_bytes": answer.selected_bytes.to_string(),
                        "remaining_bytes": answer.remaining_bytes.to_string(),
                    });
                    encoded = Some(relation_reply::encode_within_request(
                        server.clone(),
                        &data,
                        expired,
                    )?);
                    Ok(())
                },
            )?;
            out.push(encoded.ok_or(BusinessError::BudgetExceeded)?);
            Ok(())
        }
        _ => Err(EngineError::Business(BusinessError::InvalidArgument)),
    }
}
