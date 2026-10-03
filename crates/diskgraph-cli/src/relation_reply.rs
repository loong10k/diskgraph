//! 关系 CLI 的有限 envelope 编码与终态授权；按现有字段报告有界前缀。
use diskgraph_core::{
    Authorizer, BusinessError, Envelope, PrincipalId, QueryBudget, measure_json_bounded,
};
use diskgraph_engine::{Engine, EngineError};
use serde_json::{Value, json};
use std::time::Instant;

/// 在实际 JSON 文本完成编码后，复核真实 revision 权限与共同期限。
/// 参数：engine/身份/revision 为本请求，data 为有界结果，deadline 从 dispatch 入口继承。
/// 返回：已复核的 envelope 文本；不能容纳诊断或撤权时拒绝。
pub(super) fn finish(
    engine: &Engine,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    revision: &str,
    data: Value,
    deadline: Instant,
) -> Result<String, EngineError> {
    let truncated = data.get("truncated").is_some_and(|value| !value.is_null());
    let mut envelope = Envelope::ok(data).with_ids(Some(engine.server_id()?), None, None);
    envelope.truncated = truncated;
    let mut envelope = envelope.into_json();
    let encoded = encode(&envelope);
    let live = engine.finalize_revision_read_until(revision, principal, authorizer, deadline)?;
    let text = encoded?;
    if live {
        return Ok(text);
    }
    envelope["data"]["complete"] = json!(false);
    envelope["data"]["truncated"] = json!("deadline");
    envelope["truncated"] = json!(true);
    let encoded = encode(&envelope);
    engine.finalize_revision_read_until(revision, principal, authorizer, deadline)?;
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
