use serde::{Deserialize, Serialize};
/// 有限方法覆盖或缺失原因码；来源：原生 Rust D42 / EV-06，不声明全机无占用。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessObservationCode {
    VisibilityRestricted,
    PermissionDenied,
    ProcessChanged,
    TargetChanged,
    BudgetExceeded,
    Timeout,
    Cancelled,
    Unsupported,
    Unavailable,
}
impl ProcessObservationCode {
    /// 参数：无；返回：固定安全覆盖原因标签，不含原始程序文本。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VisibilityRestricted => "visibility_restricted",
            Self::PermissionDenied => "permission_denied",
            Self::ProcessChanged => "process_changed",
            Self::TargetChanged => "target_changed",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
        }
    }
}
