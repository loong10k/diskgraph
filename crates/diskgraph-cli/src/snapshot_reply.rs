//! 快照响应只在 Engine 原请求的编码回调内准备，授权完成后由适配器发布。
use diskgraph_core::{BusinessError, Envelope, QueryBudget, ServerId, measure_json_bounded};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

/// 为公开 envelope 保守预留空间，不重复扣除原始读取账本。
/// 参数：无。返回：沿用默认节点/时间预算，数据编码预留 2048 字节。
pub(super) fn budget() -> QueryBudget {
    QueryBudget {
        max_response_bytes: QueryBudget::default().max_response_bytes - 2048,
        ..QueryBudget::default()
    }
}

/// 在 Engine 连续授权请求内编码，不重新建立撤权见证，也不输出。
/// 参数：服务器身份、借用的有界数据及原数据期限状态；返回：兼容 envelope 或编码预算错误。
pub(super) fn encode_data(
    server_id: ServerId,
    data: &Value,
    expired: bool,
) -> Result<String, EngineError> {
    let mut envelope = Envelope::ok(Value::Null).with_ids(Some(server_id), None, None);
    envelope.truncated = is_truncated(data) || expired;
    let mut envelope = envelope.into_json();
    // 先序列化固定头，再复制一次已准入的数据，避免为借用编码额外复制整份结果。
    envelope["data"] = data.clone();
    if expired {
        envelope["data"]["complete"] = json!(false);
        envelope["data"]["truncation_reason"] = json!("deadline");
        if let Some(summary) = envelope["data"].get_mut("summary_is_partial") {
            *summary = json!(true);
        }
    }
    let encoded = encode(&envelope);
    #[cfg(test)]
    crate::query_terminal_tests::before_reply();
    encoded
}

/// 在同一连续授权请求内编码完整计划，不让过期或截断计划成为可用步骤。
/// 参数：服务器身份、计划 JSON 与原期限状态；返回：完整编码或预算错误，不执行计划。
pub(super) fn encode_plan_data(
    server_id: ServerId,
    data: &Value,
    expired: bool,
) -> Result<String, EngineError> {
    let encoded = encode_data(server_id, data, expired);
    if expired || is_truncated(data) {
        return Err(BusinessError::BudgetExceeded.into());
    }
    encoded
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
