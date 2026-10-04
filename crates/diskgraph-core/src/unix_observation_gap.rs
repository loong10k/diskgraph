use serde::{Deserialize, Serialize};
/// Unix 强身份未取得的固定原因；来源：原生 Rust D42 / FS-02，缺失不能当作已验证。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnixObservationGap {
    NotCaptured,
    Unsupported,
    Denied,
    Changed,
    CaptureFailed,
    TreeMismatch,
}
impl UnixObservationGap {
    /// 参数：无；返回：有限持久原因码。
    pub fn code(self) -> &'static str {
        match self {
            Self::NotCaptured => "not_captured",
            Self::Unsupported => "unsupported",
            Self::Denied => "denied",
            Self::Changed => "changed",
            Self::CaptureFailed => "capture_failed",
            Self::TreeMismatch => "tree_mismatch",
        }
    }
}
