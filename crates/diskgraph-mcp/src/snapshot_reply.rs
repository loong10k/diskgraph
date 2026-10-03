//! 双侧历史的真实 envelope 计量与末段授权；来源：DiskGraph 原生 MCP D24 契约。
use crate::McpService;
use diskgraph_core::{BusinessError, QueryBudget, measure_json_bounded};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};
use std::time::Instant;

/// 为服务 envelope 保守预留编码空间，原始读取额度仍独立计量。
/// 参数：无。返回：沿用默认期限和节点数，数据响应预留 2048 字节。
pub(super) fn budget() -> QueryBudget {
    QueryBudget {
        max_response_bytes: QueryBudget::default().max_response_bytes - 2048,
        ..QueryBudget::default()
    }
}

/// 编码后分别复核 before/after 实际归属与 token/数据库权限交集。
/// 参数：service 为当前真实身份，envelope 为响应，revisions 为历史双侧，deadline 为原请求期限。
/// 返回：有限响应及文本；撤权、过期无法确认归属或最小诊断超限时拒绝数据。
pub(super) fn finish(
    service: &McpService,
    mut envelope: Value,
    revisions: &[&str],
    deadline: Instant,
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
    let live = finalize(service, revisions, deadline)?;
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
    finalize(service, revisions, deadline)?;
    Ok((envelope, encoded?))
}

fn finalize(
    service: &McpService,
    revisions: &[&str],
    deadline: Instant,
) -> Result<bool, EngineError> {
    service.engine.finalize_revisions_read_until(
        revisions,
        service.context.principal(),
        &service.authorizer()?,
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
