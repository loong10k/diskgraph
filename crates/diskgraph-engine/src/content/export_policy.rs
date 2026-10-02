//! 控制内容结果导出，元数据模式只保留字节数；既有 IncludeDigests 模式允许显式 bytes_hex。

use super::{InspectionStop, ReadOutcome};

/// 控制内容结果导出，元数据模式只保留字节数；既有 IncludeDigests 模式允许显式 bytes_hex。
/// 来源：原生 Rust diskgraph-engine::content::ExportPolicy。
/// How an inspection may be exported. The policy is the contract: a
/// metadata-only export cannot carry bytes, so no log or error path can
/// leak them by accident (CT-04, task 8.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportPolicy {
    MetadataOnly,
    IncludeDigests,
}

impl ExportPolicy {
    /// 按既有策略导出读取结果。
    /// 参数：outcome 为读取结果。
    /// 返回：元数据 JSON；IncludeDigests 兼容模式另含显式 bytes_hex。
    /// The exportable view of a read. `MetadataOnly` drops the bytes and
    /// reports their count instead; nothing else changes.
    pub fn export_read(&self, outcome: &ReadOutcome) -> serde_json::Value {
        let mut value = serde_json::json!({
            "path_name": outcome.requested_path.file_name().map(|name| name.to_string_lossy().into_owned()),
            "offset": outcome.offset,
            "file_len": outcome.file_len,
            "read_len": outcome.bytes.len(),
            "truncated": outcome.truncated,
            "stopped": outcome.stopped.as_ref().map(|stop| match stop {
                InspectionStop::Placeholder => "placeholder",
                InspectionStop::Cancelled => "cancelled",
                InspectionStop::Unstable => "unstable",
                InspectionStop::ByteLimit => "byte_limit",
                InspectionStop::Deadline => "deadline",
                InspectionStop::PermissionRevoked => "permission_revoked",
                InspectionStop::ReadError => "read_error",
            }),
            "observed_at_unix_ms": outcome.observed_at_unix_ms,
        });
        if *self == ExportPolicy::IncludeDigests {
            // Even with digests allowed, a read exports content only through
            // the explicit bytes field a caller asked for; policy governs it.
            value["bytes_hex"] = serde_json::Value::String(hex::encode(&outcome.bytes));
        }
        value
    }
}
