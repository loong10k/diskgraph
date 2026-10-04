use crate::{Result, StoreError};
use diskgraph_core::ProcessEvidenceLimits;
use serde::Deserialize;
use serde::de::IgnoredAny;

/// 固定 16KiB 输入中的数值限额投影；来源：Rust D42，不拥有目标身份或 epoch。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessJobLimitsView {
    schema_version: u32,
    #[serde(rename = "server_id")]
    _server_id: IgnoredAny,
    #[serde(rename = "scope_id")]
    _scope_id: IgnoredAny,
    #[serde(rename = "base_revision_id")]
    _base_revision_id: IgnoredAny,
    #[serde(rename = "node_id")]
    _node_id: IgnoredAny,
    #[serde(rename = "method")]
    _method: IgnoredAny,
    #[serde(rename = "indexed_epoch")]
    _indexed_epoch: IgnoredAny,
    limits: ProcessEvidenceLimits,
}
impl ProcessJobLimitsView {
    /// 参数：已硬准入的借用输入；返回：固定数字限额，其他内容稍后仍须完整 typed/digest 验证。
    pub(crate) fn decode(raw: &str) -> Result<ProcessEvidenceLimits> {
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| StoreError::InvalidGraph("invalid Process limits projection".into()))?;
        if value.schema_version != 1 {
            return Err(StoreError::InvalidGraph(
                "invalid Process limits version".into(),
            ));
        }
        Ok(value.limits)
    }
}
