//! 树与双侧历史的有限编码及终态授权；来源：DiskGraph 原生 CLI D24 契约。
use diskgraph_core::{
    Authorizer, BusinessError, Envelope, PrincipalId, QueryBudget, measure_json_bounded,
};
use diskgraph_engine::{Engine, EngineError};
use serde_json::{Value, json};
use std::time::Instant;

/// 为公开 envelope 保守预留空间，不重复扣除原始读取账本。
/// 参数：无。返回：沿用默认节点/时间预算，数据编码预留 2048 字节。
pub(super) fn budget() -> QueryBudget {
    QueryBudget {
        max_response_bytes: QueryBudget::default().max_response_bytes - 2048,
        ..QueryBudget::default()
    }
}

/// 实际编码完成后复核树或历史双侧真实 revision；不使用客户端 scope 替代归属。
/// 参数：engine/主体/授权器/revisions 为请求上下文，data 为有界数据，deadline 为请求原期限。
/// 返回：有限 JSON 文本；撤权、不能确认归属或无法容纳诊断时拒绝。
pub(super) fn finish(
    engine: &Engine,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    revisions: &[&str],
    data: Value,
    deadline: Instant,
) -> Result<String, EngineError> {
    finish_inner(
        engine, principal, authorizer, revisions, data, deadline, false,
    )
}

/// 同步计划在实际编码后的末段过期或截断时拒绝，不返回可用 steps。
/// 参数：请求上下文与 finish 一致，data 为已完整生成的计划。
/// 返回：有限且仍有效的计划文本，或预算/权限错误。
pub(super) fn finish_plan(
    engine: &Engine,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    revisions: &[&str],
    data: Value,
    deadline: Instant,
) -> Result<String, EngineError> {
    finish_inner(
        engine, principal, authorizer, revisions, data, deadline, true,
    )
}

fn finish_inner(
    engine: &Engine,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    revisions: &[&str],
    data: Value,
    deadline: Instant,
    require_complete: bool,
) -> Result<String, EngineError> {
    let truncated = is_truncated(&data);
    let mut envelope = Envelope::ok(data).with_ids(Some(engine.server_id()?), None, None);
    envelope.truncated = truncated;
    let mut envelope = envelope.into_json();
    let encoded = encode(&envelope);
    #[cfg(test)]
    crate::query_terminal_tests::before_reply();
    let live = finalize(engine, principal, authorizer, revisions, deadline)?;
    let text = encoded?;
    if require_complete && (!live || truncated) {
        return Err(BusinessError::BudgetExceeded.into());
    }
    if live {
        return Ok(text);
    }
    envelope["data"]["complete"] = json!(false);
    envelope["data"]["truncation_reason"] = json!("deadline");
    if let Some(summary) = envelope["data"].get_mut("summary_is_partial") {
        *summary = json!(true);
    }
    envelope["truncated"] = json!(true);
    let encoded = encode(&envelope);
    finalize(engine, principal, authorizer, revisions, deadline)?;
    encoded
}

/// 复核所有实际 revision，空列表不是已授权成功。
/// 参数：engine/主体/授权器/revisions/deadline 为本请求。返回：未过期为 true 或授权失败。
pub(super) fn finalize(
    engine: &Engine,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    revisions: &[&str],
    deadline: Instant,
) -> Result<bool, EngineError> {
    engine.finalize_revisions_read_until(revisions, principal, authorizer, deadline)
}

fn is_truncated(data: &Value) -> bool {
    data.get("complete") == Some(&Value::Bool(false))
        || data.get("truncation_reason").is_some_and(Value::is_string)
        || data
            .get("truncated")
            .is_some_and(|value| value == &Value::Bool(true) || value.is_string())
        || data.get("tree").is_some_and(crate::html::tree_is_truncated)
}

fn encode(value: &Value) -> Result<String, EngineError> {
    if measure_json_bounded(value, QueryBudget::default().max_response_bytes)
        .map_err(diskgraph_store::StoreError::from)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    serde_json::to_string(value).map_err(|error| diskgraph_store::StoreError::from(error).into())
}
