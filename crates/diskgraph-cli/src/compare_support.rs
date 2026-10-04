//! CLI compare_support 的真实职责实现。
use diskgraph_core::{BusinessError, ScopeId};
use diskgraph_engine::{Engine, EngineError};

/// 保留 plan_to_json 的原生业务职责与错误语义。来源：DiskGraph CLI main::plan_to_json。
/// 参数：与原入口的 plan_to_json 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn plan_to_json(plan: &diskgraph_core::SyncPlan, limit: usize) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = plan
        .actions
        .iter()
        .take(limit)
        // The actions are plain data with derived Serialize; a failure would
        // be a bug, not a runtime condition, so the fallback keeps the
        // envelope well formed.
        .map(|action| serde_json::to_value(action).unwrap_or(serde_json::Value::Null))
        .collect();
    serde_json::json!({
        "method": plan.method.name(),
        "deletes": plan.deletes,
        "copies": plan.copies(),
        "deletions": plan.deletions(),
        "copied_bytes": plan.copied_bytes,
        "deleted_bytes": plan.deleted_bytes,
        "unresolved": plan.unresolved,
        "excluded": plan.excluded,
        "steps_shown": steps.len(),
        "steps": steps,
    })
}

/// 保留 resolve_side 的原生业务职责与错误语义。来源：DiskGraph CLI main::resolve_side。
/// 参数：与原入口的 resolve_side 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn resolve_side(
    engine: &Engine,
    revision: Option<&str>,
    scope: Option<&str>,
    side: &str,
    deadline: std::time::Instant,
) -> Result<(String, Option<ScopeId>), EngineError> {
    if let Some(revision) = revision {
        return Ok((revision.to_owned(), None));
    }
    let Some(scope) = scope else {
        eprintln!("diskgraph: the {side} side needs --{side} or --{side}-scope");
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    };
    let scope_id = ScopeId::new(scope.to_owned())
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
    engine.scope(&scope_id)?;
    let revision = engine
        .latest_revision_until(&scope_id, deadline)?
        .ok_or(EngineError::Business(BusinessError::NotIndexed))?;
    Ok((revision, Some(scope_id)))
}
