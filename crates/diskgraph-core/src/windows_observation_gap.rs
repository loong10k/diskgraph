use crate::WindowsObservationError;

/// 完整 Windows 原生观测不可用的固定原因，未知事实不以零填充。
/// 来源：DiskGraph 原生 Rust D31；无 Java 对应对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsObservationGap {
    /// 旧版本或旧接口从未捕获完整属性。
    NotCaptured,
    /// 平台、卷或对象不支持可靠观测。
    Unsupported,
    /// 属性采集失败。
    CaptureFailed,
    /// 同句柄前后属性发生变化。
    Changed,
    /// 新采样与旧树的可比事实明确冲突。
    TreeMismatch,
}

impl WindowsObservationGap {
    /// 获取稳定缺失标签。参数：无；返回：固定的小写标签。
    pub fn code(self) -> &'static str {
        match self {
            Self::NotCaptured => "not_captured",
            Self::Unsupported => "unsupported",
            Self::CaptureFailed => "capture_failed",
            Self::Changed => "changed",
            Self::TreeMismatch => "tree_mismatch",
        }
    }

    /// 严格解析缺失标签。参数：code 为原始标签；返回：缺失原因或协议错误。
    pub fn parse(code: &str) -> Result<Self, WindowsObservationError> {
        match code {
            "not_captured" => Ok(Self::NotCaptured),
            "unsupported" => Ok(Self::Unsupported),
            "capture_failed" => Ok(Self::CaptureFailed),
            "changed" => Ok(Self::Changed),
            "tree_mismatch" => Ok(Self::TreeMismatch),
            _ => Err(WindowsObservationError::InvalidTag),
        }
    }
}
