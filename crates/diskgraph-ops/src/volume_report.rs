//! volume_report：既有文件操作职责的原生 Rust 实现。

/// 分别记录处理字节、隔离保留字节与卷空闲空间观测，不等同于归因释放量。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::VolumeReport`，保留既有语义。
/// What a completed operation did to the volume, reported honestly. The three
/// numbers are deliberately separate: bytes the operation processed, bytes the
/// quarantine still holds (so the user knows the space is not free), and the
/// measured free-space delta on the volume (OP-10).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeReport {
    /// Logical bytes the operation moved, copied, quarantined, or restored.
    pub processed_bytes: u64,
    /// Bytes still held in the quarantine for this operation.
    pub retained_in_quarantine_bytes: u64,
    /// Free bytes on the volume before the operation.
    pub free_before_bytes: u64,
    /// Free bytes on the volume after the operation.
    pub free_after_bytes: u64,
    /// When the free-space measurement was taken, milliseconds since epoch.
    pub measured_at_unix_ms: u64,
    /// Why the delta must not be read as "this operation freed that much".
    pub caveat: &'static str,
}

impl VolumeReport {
    /// 计算两次卷空闲空间观测差。
    /// 参数：self 包含前后测量。
    /// 返回：原有有符号差值，不归因于单个操作。
    /// The measured difference, which is what a real user should see.
    pub fn free_delta_bytes(&self) -> i64 {
        self.free_after_bytes as i64 - self.free_before_bytes as i64
    }

    /// 序列化既有卷空间报告字段。
    /// 参数：self 为测量报告。
    /// 返回：兼容原有字段和字符串字节数的 JSON。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "processed_bytes": self.processed_bytes.to_string(),
            "retained_in_quarantine_bytes": self.retained_in_quarantine_bytes.to_string(),
            "free_before_bytes": self.free_before_bytes.to_string(),
            "free_after_bytes": self.free_after_bytes.to_string(),
            "free_delta_bytes": self.free_delta_bytes().to_string(),
            "measured_at_unix_ms": self.measured_at_unix_ms,
            "caveat": self.caveat,
        })
    }
}
