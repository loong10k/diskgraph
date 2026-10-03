//! MCP 业务错误诊断的实际编码门禁；来源：DiskGraph Q-02/D24 原生协议契约。
use diskgraph_core::{BusinessError, QueryBudget, measure_json_bounded};
use serde_json::Value;

/// 有界业务诊断保留业务码、退出码及 JSON-RPC 关联，超额时使用有限信息。
/// 参数：id 为协议关联，error/message 为真实失败。返回：原错误帧形状，无成功数据。
/// JSON-RPC ID 及 HTTP/SSE 外包装另受传输预算约束；这里限制业务诊断并预留 1024 字节。
pub(super) fn tool_error(id: &Value, error: BusinessError, message: &str) -> Value {
    let frame = crate::protocol::tool_error(id, error, message);
    if matches!(
        measure_json_bounded(
            &frame["error"],
            QueryBudget::default().max_response_bytes - 1024
        ),
        Ok(Some(_))
    ) {
        frame
    } else {
        crate::protocol::tool_error(id, error, "diagnostic exceeds response byte budget")
    }
}
