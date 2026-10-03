use crate::WindowsObservationError;

/// 独立原生采样与旧扫描树的可比事实对齐状态；不证明内容相同。
/// 来源：DiskGraph 原生 Rust D31；无 Java 对应对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsTreeAlignment {
    /// 能够比较的旧身份、类型及尺寸条件一致。
    Matched,
    /// 旧身份不足或尺寸口径不可比。
    Unverified,
}

impl WindowsTreeAlignment {
    /// 获取稳定协议标签。参数：无；返回：固定的小写标签。
    pub fn code(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Unverified => "unverified",
        }
    }

    /// 严格解析协议标签。参数：code 为原始标签；返回：状态或未知标签错误。
    pub fn parse(code: &str) -> Result<Self, WindowsObservationError> {
        match code {
            "matched" => Ok(Self::Matched),
            "unverified" => Ok(Self::Unverified),
            _ => Err(WindowsObservationError::InvalidTag),
        }
    }
}
