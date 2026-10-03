//! cleanup_result：既有文件操作职责的原生 Rust 实现。

/// Docker 精确对象清理后的保守结果，保留虚拟机空间限制说明。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::docker::CleanupResult`，保留既有语义。
/// One object's cleanup result, reported separately (7.9, OP-09).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupResult {
    /// Docker's own output proved the removal.
    Confirmed,
    /// The object was in use and was deliberately skipped.
    SkippedInUse { reason: String },
    /// The removal could not be proven: park for reconciliation (OP-08).
    NeedsAttention { reason: String },
}
