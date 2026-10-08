//! 关系工具的真实 envelope 编码与末段实时授权，不向共享服务写入请求状态。
use crate::McpService;
use diskgraph_core::{BusinessError, QueryBudget, measure_json_bounded};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};
#[cfg(test)]
use std::time::Instant;

/// 在 Engine 原请求内计量并编码，禁止重新捕获授权见证。
/// 参数：当前请求服务、实际 envelope 及原期限状态；返回：终检后才能发布的文本。
pub(super) fn encode_within_request(
    service: &McpService,
    envelope: &mut Value,
    expired: bool,
) -> Result<String, EngineError> {
    if expired {
        envelope["data"]["complete"] = json!(false);
        envelope["data"]["truncated"] = json!("deadline");
    }
    envelope["truncated"] = json!(
        envelope["data"]["complete"] == false
            || envelope["data"]["truncated"].is_string()
            || envelope["data"]["truncated"] == true
    );
    let encoded = encode(envelope, QueryBudget::default().max_response_bytes);
    #[cfg(test)]
    crate::relation_budget_tests::before_reply(service);
    #[cfg(not(test))]
    let _ = service;
    encoded
}

/// 编码完成后确认真实 revision 的实时权限；到期保留合法前缀并报告 deadline。
/// 参数：service 为当前身份，envelope 为实际响应，deadline 在 tools/call 入口创建。
/// authorizer 为同次请求有界捕获的能力上限；持久授权仍实时复验，不跨请求缓存。
/// 返回：有限 envelope 和对应文本；无法容纳完整诊断或撤权时返回失败。
#[cfg(test)]
pub(super) fn finish(
    service: &McpService,
    mut envelope: Value,
    deadline: Instant,
    authorizer: &dyn diskgraph_core::Authorizer,
) -> Result<(Value, String), EngineError> {
    let revision = envelope["revision_id"]
        .as_str()
        .ok_or(BusinessError::InvalidArgument)?
        .to_owned();
    let cap = QueryBudget::default().max_response_bytes;
    let encoded = encode(&envelope, cap);
    #[cfg(test)]
    crate::relation_budget_tests::before_reply(service);
    let live = service.engine.finalize_revision_read_until(
        &revision,
        service.context.principal(),
        authorizer,
        deadline,
    )?;
    let text = encoded?;
    if live {
        return Ok((envelope, text));
    }
    envelope["data"]["complete"] = json!(false);
    envelope["data"]["truncated"] = json!("deadline");
    envelope["truncated"] = json!(true);
    let encoded = encode(&envelope, cap);
    // 到期诊断的编码也必须完成后复核；不能借 deadline 跳过撤权或 token 到期。
    service.engine.finalize_revision_read_until(
        &revision,
        service.context.principal(),
        authorizer,
        deadline,
    )?;
    Ok((envelope, encoded?))
}

fn encode(value: &Value, cap: usize) -> Result<String, EngineError> {
    if measure_json_bounded(value, cap)
        .map_err(diskgraph_store::StoreError::from)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    serde_json::to_string(value).map_err(|error| diskgraph_store::StoreError::from(error).into())
}
