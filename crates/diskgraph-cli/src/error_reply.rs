//! CLI 业务失败的有限诊断；来源：DiskGraph Q-02/D24 原生响应契约。
use diskgraph_core::{Envelope, QueryBudget, measure_json_bounded};
use diskgraph_engine::EngineError;

/// 按实际 JSON 转义限制错误 envelope，保留原业务错误码及退出码。
/// 参数：error 为真实执行失败。返回：有限失败 JSON，永远不含成功数据。
pub(super) fn line(error: &EngineError) -> String {
    let business = super::engine_business(error);
    let mut envelope = Envelope::failure(business, error.to_string());
    if !matches!(
        measure_json_bounded(&envelope, QueryBudget::default().max_response_bytes),
        Ok(Some(_))
    ) {
        envelope = Envelope::failure(business, "diagnostic exceeds response byte budget");
    }
    envelope.into_json().to_string()
}
