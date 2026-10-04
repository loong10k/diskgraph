//! Git 固定合同的严格字段解码；来源：原生 Rust EC-02 / RT-01。
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// 参数：value 为原始对象，keys 为唯一允许字段；返回：完整精确对象或拒绝。
pub(crate) fn object<'a>(
    value: &'a Value,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, &'static str> {
    let object = value.as_object().ok_or("Git contract must be an object")?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err("unexpected or missing Git contract field");
    }
    Ok(object)
}

/// 参数：object 为已核对的字段表，name 为必需字段；返回：严格 typed 值或解码错误。
pub(crate) fn field<T: DeserializeOwned>(
    object: &Map<String, Value>,
    name: &str,
) -> Result<T, String> {
    serde_json::from_value(object.get(name).ok_or("missing Git field")?.clone())
        .map_err(|_| format!("invalid Git field: {name}"))
}

/// 参数：value 为服务端键；返回：有限 ASCII 标识可持久化，否则拒绝。
pub(crate) fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
