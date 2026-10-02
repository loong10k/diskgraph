//! 记录进程占用样本与覆盖说明，仅完整空样本说明未观察到句柄。

use super::{ProcessHolder, UsageCoverage};

/// 记录进程占用样本与覆盖说明，仅完整空样本说明未观察到句柄。
/// 来源：原生 Rust diskgraph-engine::live_evidence::UsageSample。
/// One sample of who is using the named paths, with the visibility caveat
/// attached. Even `Full` describes observations within a verified scope, never a
/// deletion authorization. Partial coverage retains positive observations,
/// but an empty partial sample leaves usage unknown (EV-06).
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
                if self.holders.is_empty() {
                    format!("usage is unknown: {reason}")
                } else {
                    format!(
                        "{} observed process(es) hold open handles; coverage is partial: {reason}",
                        self.holders.len()
                    )
                }
            }
            UsageCoverage::Unobservable { reason } => {
                format!("usage is unobservable: {reason}")
            }
        }
    }
}
