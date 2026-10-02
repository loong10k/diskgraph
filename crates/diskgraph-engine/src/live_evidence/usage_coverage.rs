//! 描述进程占用采样的可见范围，无法观察不能报告无人占用。

/// 描述进程占用采样的可见范围，无法观察不能报告无人占用。
/// 来源：原生 Rust diskgraph-engine::live_evidence::UsageCoverage。
/// How much of the system the sample could actually see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsageCoverage {
    /// 在经明确验证的权限与观察范围内回答全部查询对象；不代表删除许可。
    Full,
    /// 保留可见正向观察，但权限范围或进程启动身份尚未完整核验。
    Partial { reason: String },
    /// The probe could not run or could not answer at all.
    Unobservable { reason: String },
}
