use crate::IndexedFileEpoch;
use crate::process_evidence_codec::{field, object};
use serde::{Deserialize, Deserializer, Serialize};
/// 扫描句柄捕获的 Unix 普通文件版本及可靠历代身份；来源：原生 Rust D42 / FS-02。
/// 旧节点 dev/inode/秒级 mtime 不足以构造该类型；非零 generation 仍须 native 证明来源。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnixFileObservation {
    schema_version: u32,
    epoch: IndexedFileEpoch,
    file_mode: u32,
    length: u64,
    modified: (i64, u32),
    changed: (i64, u32),
    capture_window: (u64, u64),
}
impl UnixFileObservation {
    /// 参数：扫描历代身份、模式、长度、修改/变更纳秒和窗口；返回：可持久普通文件观察。
    pub fn new(
        epoch: IndexedFileEpoch,
        file_mode: u32,
        length: u64,
        modified: (i64, u32),
        changed: (i64, u32),
        capture_window: (u64, u64),
    ) -> Result<Self, &'static str> {
        let value = Self {
            schema_version: 1,
            epoch,
            file_mode,
            length,
            modified,
            changed,
            capture_window,
        };
        value.validate()?;
        Ok(value)
    }
    /// 参数：无；返回：严格版本、普通文件与可靠 Unix 身份校验。
    pub fn validate(&self) -> Result<(), &'static str> {
        self.epoch.validate()?;
        if self.schema_version != 1
            || matches!(self.epoch, IndexedFileEpoch::Windows { .. })
            || self.file_mode & 0xf000 != 0x8000
            || self.length > i64::MAX as u64
            || self.modified.1 >= 1_000_000_000
            || self.changed.1 >= 1_000_000_000
            || self.capture_window.0 == 0
            || self.capture_window.1 < self.capture_window.0
            || self.capture_window.1 > i64::MAX as u64
        {
            return Err("invalid Unix file observation");
        }
        Ok(())
    }
    /// 参数：无；返回：扫描时捕获身份。
    pub fn epoch(&self) -> &IndexedFileEpoch {
        &self.epoch
    }
    /// 参数：无；返回：文件字节长度，不是目录聚合大小。
    pub fn length(&self) -> u64 {
        self.length
    }
    /// 参数：无；返回：保存原纳秒精度的修改时间。
    pub fn modified(&self) -> (i64, u32) {
        self.modified
    }
    /// 参数：无；返回：保存原纳秒精度的 inode 变更时间。
    pub fn changed(&self) -> (i64, u32) {
        self.changed
    }
    /// 参数：无；返回：实际捕获窗口。
    pub fn capture_window(&self) -> (u64, u64) {
        self.capture_window
    }
    /// 参数：无；返回：有限严格版本 JSON 或无效观察错误。
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        self.validate()?;
        let raw = serde_json::to_vec(self).map_err(|_| "invalid Unix encoding")?;
        if raw.len() > 1024 {
            return Err("oversized Unix observation");
        }
        Ok(raw)
    }
    /// 参数：完整原始记录；返回：准入后重验的观察，损坏和未知版本明确拒绝。
    pub fn decode(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() > 1024 {
            return Err("oversized Unix observation");
        }
        let v: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid Unix observation encoding")?;
        v.validate()?;
        Ok(v)
    }
}

impl<'de> Deserialize<'de> for UnixFileObservation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(deserializer)?;
        let o = object(
            &v,
            &[
                "schema_version",
                "epoch",
                "file_mode",
                "length",
                "modified",
                "changed",
                "capture_window",
            ],
        )
        .map_err(serde::de::Error::custom)?;
        if field::<u32>(o, "schema_version").map_err(serde::de::Error::custom)? != 1 {
            return Err(serde::de::Error::custom(
                "unsupported Unix observation version",
            ));
        }
        Self::new(
            field(o, "epoch").map_err(serde::de::Error::custom)?,
            field(o, "file_mode").map_err(serde::de::Error::custom)?,
            field(o, "length").map_err(serde::de::Error::custom)?,
            field(o, "modified").map_err(serde::de::Error::custom)?,
            field(o, "changed").map_err(serde::de::Error::custom)?,
            field(o, "capture_window").map_err(serde::de::Error::custom)?,
        )
        .map_err(serde::de::Error::custom)
    }
}
