use crate::git_evidence_codec::{field, object};
use serde::{Deserialize, Deserializer, Serialize};

/// 服务端持久化的 Git 任务限额，不允许客户端提供；来源：原生 Rust EC-04 / D35。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GitEvidenceLimits {
    max_duration_ms: u64,
    max_output_bytes: u64,
    max_input_bytes: u64,
    max_input_entries: u64,
    max_capture_bytes: u64,
    min_free_bytes: u64,
}

impl Default for GitEvidenceLimits {
    fn default() -> Self {
        Self {
            max_duration_ms: 15_000,
            max_output_bytes: 1 << 20,
            max_input_bytes: 64 << 20,
            max_input_entries: 32768,
            max_capture_bytes: 128 << 20,
            min_free_bytes: 64 << 20,
        }
    }
}

impl GitEvidenceLimits {
    /// 参数：六项为服务端有限时间、输出、输入字节/条目、捕获及剩余空间配置；返回：有效配置或拒绝。
    pub fn new(
        max_duration_ms: u64,
        max_output_bytes: u64,
        max_input_bytes: u64,
        max_input_entries: u64,
        max_capture_bytes: u64,
        min_free_bytes: u64,
    ) -> Result<Self, &'static str> {
        if max_duration_ms == 0
            || max_duration_ms > 300_000
            || max_output_bytes == 0
            || max_output_bytes > 64 << 20
            || max_input_bytes == 0
            || max_input_bytes > 64 << 20
            || max_input_entries == 0
            || max_input_entries > 32768
            || max_capture_bytes == 0
            || max_capture_bytes > 128 << 20
            || min_free_bytes < 64 << 20
            || min_free_bytes > i64::MAX as u64
        {
            return Err("invalid Git evidence limits");
        }
        Ok(Self {
            max_duration_ms,
            max_output_bytes,
            max_input_bytes,
            max_input_entries,
            max_capture_bytes,
            min_free_bytes,
        })
    }
    /// 参数：无；返回：从认领开始的总运行毫秒额度。
    pub fn max_duration_ms(&self) -> u64 {
        self.max_duration_ms
    }
    /// 参数：无；返回：所有子进程共享的输出字节额度。
    pub fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }
    /// 参数：无；返回：元数据输入累计字节额度。
    pub fn max_input_bytes(&self) -> u64 {
        self.max_input_bytes
    }
    /// 参数：无；返回：元数据输入累计条目额度。
    pub fn max_input_entries(&self) -> u64 {
        self.max_input_entries
    }
    /// 参数：无；返回：私有捕获累计字节额度。
    pub fn max_capture_bytes(&self) -> u64 {
        self.max_capture_bytes
    }
    /// 参数：无；返回：捕获卷最小剩余字节。
    pub fn min_free_bytes(&self) -> u64 {
        self.min_free_bytes
    }
}
impl<'de> Deserialize<'de> for GitEvidenceLimits {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let keys = [
            "max_duration_ms",
            "max_output_bytes",
            "max_input_bytes",
            "max_input_entries",
            "max_capture_bytes",
            "min_free_bytes",
        ];
        let o = object(&value, &keys).map_err(serde::de::Error::custom)?;
        let read =
            |name| -> Result<u64, D::Error> { field(o, name).map_err(serde::de::Error::custom) };
        Self::new(
            read(keys[0])?,
            read(keys[1])?,
            read(keys[2])?,
            read(keys[3])?,
            read(keys[4])?,
            read(keys[5])?,
        )
        .map_err(serde::de::Error::custom)
    }
}
