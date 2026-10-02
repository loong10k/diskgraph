//! 描述进程占用采样的可见范围，无法观察不能报告无人占用。

/// 描述进程占用采样的可见范围，无法观察不能报告无人占用。
/// 来源：原生 Rust diskgraph-engine::live_evidence::UsageCoverage。
/// How much of the system the sample could actually see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsageCoverage {
    /// The probe ran and answered for every named path.
    Full,
    /// The probe ran but could not see everything (partial permissions).
    Partial { reason: String },
    /// The probe could not run or could not answer at all.
    Unobservable { reason: String },
}
