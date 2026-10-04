use serde::{Deserialize, Serialize};
/// 有限方法覆盖或缺失原因码；来源：原生 Rust D42 / EV-06，不声明全机无占用。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessObservationCoverage {
    VisibleMethodDomainComplete,
    Partial,
}
