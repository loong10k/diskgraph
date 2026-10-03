//! 既有 JSON envelope 与列表大小约束。
use crate::api_result::ApiResult;
use serde_json::json;
const MAX_QUERY_LIMIT: u32 = 1_000;

/// 校验旧列表 API 的条数范围。
/// 参数：limit 为请求条数。
/// 返回：1 到 1000 内的原值，超范围返回错误。
pub(crate) fn bounded_limit(limit: u32) -> Result<u32, String> {
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        Err(format!("limit must be between 1 and {MAX_QUERY_LIMIT}"))
    } else {
        Ok(limit)
    }
}

/// 编码既有 schema_version=1 的成功或错误 envelope。
/// 参数：result 为 JSON 数据或可展示错误。
/// 返回：完整 JSON 字符串。
pub(crate) fn response(result: ApiResult) -> String {
    match result {
        Ok(data) => json!({ "schema_version": 1, "ok": true, "data": data }).to_string(),
        Err(error) => json!({ "schema_version": 1, "ok": false, "error": error }).to_string(),
    }
}
