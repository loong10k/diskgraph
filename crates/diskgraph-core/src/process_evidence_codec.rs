//! Process 固定协议的严格字段校验；来源：原生 Rust D42 / EV-05。
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// 参数：解码对象与协议全部字段；返回：精确匹配的借用对象，额外和缺失字段均拒绝。
pub(crate) fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, &'static str> {
    let object = value
        .as_object()
        .ok_or("process contract must be an object")?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err("unexpected or missing process contract field");
    }
    Ok(object)
}
/// 参数：已校验对象及固定字段名；返回：重新校验的字段值或固定字段格式错误。
pub(crate) fn field<T: DeserializeOwned>(
    object: &Map<String, Value>,
    name: &str,
) -> Result<T, String> {
    serde_json::from_value(object.get(name).ok_or("missing process field")?.clone())
        .map_err(|_| format!("invalid process field: {name}"))
}
/// 参数：内部持久标识；返回：是否为长度有限且不含路径/控制字符的键。
pub(crate) fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
