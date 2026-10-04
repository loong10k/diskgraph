use serde::{Deserialize, Deserializer, Serialize};
/// 认领开始全程共享的服务端占用观察限额；来源：原生 Rust D42 / EC-04。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProcessEvidenceLimits {
    max_duration_ms: u64,
    max_metadata_bytes: u64,
    max_entries: u64,
    max_result_bytes: u64,
    max_allocation_bytes: u64,
    max_retries: u32,
    max_handles: u32,
}
impl Default for ProcessEvidenceLimits {
    fn default() -> Self {
        Self {
            max_duration_ms: 15000,
            max_metadata_bytes: 4 << 20,
            max_entries: 32768,
            max_result_bytes: 65536,
            max_allocation_bytes: 8 << 20,
            max_retries: 2,
            max_handles: 64,
        }
    }
}
impl ProcessEvidenceLimits {
    /// 参数：服务端七项时间、元数据/条目/结果/分配、重试与句柄上限；返回：严格有限配置。
    pub fn new(
        max_duration_ms: u64,
        max_metadata_bytes: u64,
        max_entries: u64,
        max_result_bytes: u64,
        max_allocation_bytes: u64,
        max_retries: u32,
        max_handles: u32,
    ) -> Result<Self, &'static str> {
        let value = Self {
            max_duration_ms,
            max_metadata_bytes,
            max_entries,
            max_result_bytes,
            max_allocation_bytes,
            max_retries,
            max_handles,
        };
        value.validate()?;
        Ok(value)
    }
    /// 参数：无；返回：所有上限在协议范围内，不保证同步原生调用硬中断或 RSS。
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=300000).contains(&self.max_duration_ms)
            || !(1..=64 << 20).contains(&self.max_metadata_bytes)
            || !(1..=1_000_000).contains(&self.max_entries)
            || !(1..=1 << 20).contains(&self.max_result_bytes)
            || !(1..=128 << 20).contains(&self.max_allocation_bytes)
            || self.max_allocation_bytes < self.max_result_bytes
            || self.max_retries > 8
            || !(1..=1024).contains(&self.max_handles)
        {
            return Err("invalid process evidence limits");
        }
        Ok(())
    }
    /// 参数：无；返回：服务端持久的 max_duration_ms 原额度。
    pub fn max_duration_ms(&self) -> u64 {
        self.max_duration_ms
    }
    /// 参数：无；返回：服务端持久的 max_metadata_bytes 原额度。
    pub fn max_metadata_bytes(&self) -> u64 {
        self.max_metadata_bytes
    }
    /// 参数：无；返回：服务端持久的 max_entries 原额度。
    pub fn max_entries(&self) -> u64 {
        self.max_entries
    }
    /// 参数：无；返回：服务端持久的 max_result_bytes 原额度。
    pub fn max_result_bytes(&self) -> u64 {
        self.max_result_bytes
    }
    /// 参数：无；返回：服务端持久的 max_allocation_bytes 原额度。
    pub fn max_allocation_bytes(&self) -> u64 {
        self.max_allocation_bytes
    }
    /// 参数：无；返回：服务端持久的 max_retries 原额度。
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }
    /// 参数：无；返回：服务端持久的 max_handles 原额度。
    pub fn max_handles(&self) -> u32 {
        self.max_handles
    }
}
impl<'de> Deserialize<'de> for ProcessEvidenceLimits {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        crate::process_evidence_limits_fields::ProcessEvidenceLimitsFields::deserialize(
            deserializer,
        )?
        .into_limits()
        .map_err(serde::de::Error::custom)
    }
}
