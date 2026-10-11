//! CLI节点窄读共用原期限、实际revision归属与编码后的实时授权终检。
use crate::cli::Cli;
use crate::command::Command;
use crate::snapshot_reply;
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, QueryReadBudget, ScopeId};
use diskgraph_engine::{Engine, EngineError};
use std::time::Instant;

/// 执行节点和目录页查询，编码成功且终检允许后才发布响应。
/// 参数：原CLI命令、真实主体及能力、输出缓冲与dispatch原期限。返回：结果或预算/授权错误。
pub(crate) fn run(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
    deadline: Instant,
) -> Result<(), EngineError> {
    let scope = match &cli.command {
        Command::Node { scope }
        | Command::Children { scope, .. }
        | Command::Top { scope, .. }
        | Command::Explore { scope, .. } => scope,
        _ => return Err(BusinessError::InvalidArgument.into()),
    };
    let scope = ScopeId::new(scope.clone()).map_err(|_| BusinessError::InvalidArgument)?;
    let revision = engine
        .latest_revision_until(&scope, deadline)?
        .ok_or(BusinessError::NotIndexed)?;
    let server = engine.server_id_until(deadline)?;
    let encoded = engine.with_authorized_revision_reader_for_scope_until(
        &revision, &scope, principal, authorizer, deadline, |reader, snapshot, original| {
            let mut budget = snapshot_reply::budget();
            if let Command::Explore { max_nodes, .. } = &cli.command {
                budget.max_nodes = (*max_nodes).clamp(1, budget.max_nodes);
            }
            let mut reads = QueryReadBudget::new(budget, original)?;
            let data = match &cli.command {
                Command::Node { .. } => {
                    let node = reader.root_node_with_budget(snapshot, &mut reads)?.ok_or(BusinessError::NotFound)?;
                    let coverage = reader.snapshot_with_budget(snapshot, &mut reads)?.coverage;
                    serde_json::json!({ "revision_id": revision, "node": node, "coverage": coverage })
                }
                Command::Children { parent_id, limit, offset, min_bytes, unknown_only, .. } => {
                    if *unknown_only && min_bytes.is_some() { return Err(BusinessError::InvalidArgument.into()); }
                    let (items, next_offset, unknown_count) = if *unknown_only {
                        let (items, next) = reader.unknown_children_page_with_budget(snapshot, *parent_id, *offset, (*limit).min(100), &mut reads)?;
                        (items, next, 0)
                    } else { reader.children_page_with_budget(snapshot, *parent_id, *min_bytes, *offset, (*limit).min(100), &mut reads)? };
                    serde_json::json!({ "revision_id": revision, "size_kind": "allocated", "items": items, "next_offset": next_offset, "unknown_size_count": unknown_count })
                }
                Command::Top { parent_id, limit, .. } => {
                    let limit = (*limit).min(100);
                    let (items, more, _) = reader.children_with_budget(snapshot, *parent_id, 0, limit, false, &mut reads)?;
                    serde_json::json!({ "revision_id": revision, "size_kind": "allocated", "items": items, "truncated": more.then_some("node_limit") })
                }
                Command::Explore { node_id, max_nodes, .. } => {
                    let limit = (*max_nodes).min(100).saturating_sub(1);
                    let node = reader.node_with_budget(snapshot, *node_id, &mut reads)?.ok_or(BusinessError::NotFound)?;
                    let (children, more, _) = reader.children_with_budget(snapshot, *node_id, 0, limit as u64, false, &mut reads)?;
                    let coverage = reader.snapshot_with_budget(snapshot, &mut reads)?.coverage;
                    serde_json::json!({ "revision_id": revision, "node": node, "children": children, "coverage": coverage, "truncated": (more || limit == 0).then_some("node_limit") })
                }
                _ => return Err(BusinessError::InvalidArgument.into()),
            };
            // 预算停止不能被误标为普通页末；本入口未定义可恢复raw截断游标，失败关闭。
            if reads.stopped().is_some() { return Err(BusinessError::BudgetExceeded.into()); }
            snapshot_reply::encode_data(server, &data, Instant::now() >= original)
        },
    )?;
    out.push(encoded);
    Ok(())
}
