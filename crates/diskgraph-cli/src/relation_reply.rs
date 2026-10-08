//! 关系 CLI 的有限 envelope 编码与终态授权；按现有字段报告有界前缀。
use diskgraph_core::{BusinessError, Envelope, QueryBudget, ServerId, measure_json_bounded};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

/// 在 Engine 原请求内编码有界结果；不得重新建立撤权见证或直接输出。
/// 参数：服务器身份、有界结果及原请求期限状态；返回：终态授权通过后才可发布的文本。
pub(super) fn encode_within_request(
    server_id: ServerId,
    data: &Value,
    expired: bool,
) -> Result<String, EngineError> {
    let mut envelope = Envelope::ok(Value::Null)
        .with_ids(Some(server_id), None, None)
        .into_json();
    envelope["data"] = data.clone();
    if expired {
        envelope["data"]["complete"] = json!(false);
        envelope["data"]["truncated"] = json!("deadline");
    }
    envelope["truncated"] = json!(
        envelope["data"]["complete"] == false
            || envelope["data"]["truncated"].is_string()
            || envelope["data"]["truncated"] == true
    );
    let encoded = encode(&envelope);
    #[cfg(test)]
    crate::query_terminal_tests::before_reply();
    encoded
}

fn encode(envelope: &Value) -> Result<String, EngineError> {
    if measure_json_bounded(envelope, QueryBudget::default().max_response_bytes)
        .map_err(diskgraph_store::StoreError::from)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    serde_json::to_string(envelope).map_err(|error| diskgraph_store::StoreError::from(error).into())
}
