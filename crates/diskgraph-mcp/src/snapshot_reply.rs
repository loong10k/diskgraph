//! 双侧历史的真实 envelope 计量与末段授权；来源：DiskGraph 原生 MCP D24 契约。
use crate::McpService;
use diskgraph_core::{BusinessError, QueryBudget, measure_json_bounded};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};
#[cfg(test)]
use std::time::Instant;

/// 为服务 envelope 保守预留编码空间，原始读取额度仍独立计量。
/// 参数：无。返回：沿用默认期限和节点数，数据响应预留 2048 字节。
pub(super) fn budget() -> QueryBudget {
    QueryBudget {
        max_response_bytes: QueryBudget::default().max_response_bytes - 2048,
        ..QueryBudget::default()
    }
}

/// 在 Engine 原请求内编码历史 envelope，沿用同一撤权见证和期限。
/// 参数：service 为固定请求身份，envelope 为有界数据，expired 为原查询期限状态。
/// 返回：待授权完成后才能发布的文本，并原地更新期限诊断；不重新捕获授权见证。
pub(super) fn encode_within_request(
    service: &McpService,
    envelope: &mut Value,
    expired: bool,
) -> Result<String, EngineError> {
    envelope["truncated"] = json!(
        expired
            || envelope["data"]["complete"] == false
            || envelope["data"]["truncation_reason"].is_string()
            || envelope["data"]["truncated"].is_string()
            || envelope["data"]["truncated"] == true
    );
    if expired {
        envelope["data"]["complete"] = json!(false);
        envelope["data"]["truncated"] = json!("deadline");
        envelope["data"]["truncation_reason"] = json!("deadline");
        if let Some(summary) = envelope["data"].get_mut("summary_is_partial") {
            *summary = json!(true);
        }
    }
    let encoded = encode(envelope);
    #[cfg(test)]
    crate::history_budget_tests::before_reply(service);
    #[cfg(not(test))]
    let _ = service;
    encoded
}

/// 编码后分别复核 before/after 实际归属与 token/数据库权限交集。
/// 参数：service 为当前真实身份，envelope 为响应，revisions 为历史双侧，deadline 为原请求期限。
/// authorizer 为同次请求有界捕获的能力上限；持久授权仍实时复验，不跨请求缓存。
/// 返回：有限响应及文本；撤权、过期无法确认归属或最小诊断超限时拒绝数据。
#[cfg(test)]
pub(super) fn finish(
    service: &McpService,
    mut envelope: Value,
    revisions: &[&str],
    deadline: Instant,
    authorizer: &dyn diskgraph_core::Authorizer,
) -> Result<(Value, String), EngineError> {
    envelope["truncated"] = json!(
        envelope["data"]["complete"] == false
            || envelope["data"]["truncation_reason"].is_string()
            || envelope["data"]["truncated"].is_string()
            || envelope["data"]["truncated"] == true
    );
    let encoded = encode(&envelope);
    #[cfg(test)]
    crate::history_budget_tests::before_reply(service);
    let live = finalize(service, revisions, deadline, authorizer)?;
    let text = encoded?;
    if live {
        return Ok((envelope, text));
    }
    envelope["data"]["complete"] = json!(false);
    envelope["data"]["truncated"] = json!("deadline");
    envelope["data"]["truncation_reason"] = json!("deadline");
    if let Some(summary) = envelope["data"].get_mut("summary_is_partial") {
        *summary = json!(true);
    }
    envelope["truncated"] = json!(true);
    let encoded = encode(&envelope);
    finalize(service, revisions, deadline, authorizer)?;
    Ok((envelope, encoded?))
}

#[cfg(test)]
fn finalize(
    service: &McpService,
    revisions: &[&str],
    deadline: Instant,
    authorizer: &dyn diskgraph_core::Authorizer,
) -> Result<bool, EngineError> {
    service.engine.finalize_revisions_read_until(
        revisions,
        service.context.principal(),
        authorizer,
        deadline,
    )
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
