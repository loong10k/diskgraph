use serde::{Deserialize, Deserializer};

/// Cancel 的闭合空正文；来源：serde 内部标记枚举的结构体字段校验，保留公开 unit variant。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelBody {}

impl CancelBody {
    /// 参数：deserializer 为已识别 Cancel 标签后的原正文反序列化器。
    /// 返回：仅空正文转换为 unit；未知字段保留真实 serde 错误，不复制为通用 Value。
    pub(crate) fn deserialize_unit<'de, D>(deserializer: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::deserialize(deserializer).map(|_| ())
    }
}
