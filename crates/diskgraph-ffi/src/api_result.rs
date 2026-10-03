/// 绑定层尚未编码的 JSON 成功值或可展示错误。
/// 来源：DiskGraph 原生 Rust UniFFI 只读 API；无 Java 对应对象。
pub(crate) type ApiResult = Result<serde_json::Value, String>;
