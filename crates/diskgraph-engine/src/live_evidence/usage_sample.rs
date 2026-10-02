//! 记录进程占用样本与覆盖说明，仅完整空样本说明未观察到句柄。

use super::{ProcessHolder, UsageCoverage};

/// 记录进程占用样本与覆盖说明，仅完整空样本说明未观察到句柄。
/// 来源：原生 Rust diskgraph-engine::live_evidence::UsageSample。
/// One sample of who is using the named paths, with the visibility caveat
/// attached. Only `Full` coverage with an empty holder list means "nobody
/// is using it"; any other coverage means "unknown", whatever the list
/// holds (EV-06).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageSample {
    pub sampled_at_unix_ms: u64,
    pub coverage: UsageCoverage,
    pub holders: Vec<ProcessHolder>,
}

impl UsageSample {
    /// 生成占用可见性说明，不推断可删除。
    /// 参数：无。
    /// 返回：对完整/部分/不可观察样本的实际说明。
    /// The honest review phrasing: never "safe to remove".
    pub fn verdict(&self) -> String {
        match &self.coverage {
            UsageCoverage::Full if self.holders.is_empty() => {
                "no open handles were visible to the probe".into()
            }
            UsageCoverage::Full => format!("{} process(es) hold open handles", self.holders.len()),
            UsageCoverage::Partial { reason } => {
                format!("the probe saw only part of the system: {reason}")
            }
            UsageCoverage::Unobservable { reason } => {
                format!("usage is unobservable: {reason}")
            }
        }
    }
}
