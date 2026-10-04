use serde::{Deserialize, Serialize};

/// 有限业务失败类别，不从原始 Git 错误字符串猜测原因；来源：原生 Rust EC-04 / RT-03。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitEvidenceFailureCode {
    BudgetExceeded,
    Timeout,
    Cancelled,
    PermissionDenied,
    Conflict,
    Unsupported,
    Unavailable,
    InternalError,
}
impl GitEvidenceFailureCode {
    /// 参数：无；返回：固定业务码，Conflict 不冒充精确 owner 或权限原因。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BudgetExceeded => "budget_exceeded",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::PermissionDenied => "permission_denied",
            Self::Conflict => "conflict",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::InternalError => "internal_error",
        }
    }
}
