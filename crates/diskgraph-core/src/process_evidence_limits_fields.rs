use crate::ProcessEvidenceLimits;
use serde::Deserialize;

/// 七个固定数值字段的流式解码；来源：原生 Rust D42，避免 bootstrap 先构造 JSON 树。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessEvidenceLimitsFields {
    max_duration_ms: u64,
    max_metadata_bytes: u64,
    max_entries: u64,
    max_result_bytes: u64,
    max_allocation_bytes: u64,
    max_retries: u32,
    max_handles: u32,
}
impl ProcessEvidenceLimitsFields {
    /// 参数：已按字段类型解码的七项额度；返回：原协议校验后的限额或真实无效错误。
    pub(crate) fn into_limits(self) -> Result<ProcessEvidenceLimits, &'static str> {
        ProcessEvidenceLimits::new(
            self.max_duration_ms,
            self.max_metadata_bytes,
            self.max_entries,
            self.max_result_bytes,
            self.max_allocation_bytes,
            self.max_retries,
            self.max_handles,
        )
    }
}
