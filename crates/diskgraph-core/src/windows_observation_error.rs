/// Windows 完整原生观测的版本和字段校验错误。
/// 来源：DiskGraph 原生 Rust D31；无 Java 对应对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsObservationError {
    /// 固定协议长度不符。
    InvalidLength,
    /// 未支持的协议版本。
    UnknownVersion,
    /// 布尔、对齐或缺失标签不属于固定协议。
    InvalidTag,
    /// 采样结束早于开始。
    InvalidWindow,
    /// EOF 超过 Windows 有符号长度范围。
    InvalidFileLength,
    /// 目录属性与对象类型不一致。
    InconsistentDirectory,
}

impl std::fmt::Display for WindowsObservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLength => "invalid Windows observation byte length",
            Self::UnknownVersion => "unsupported Windows observation version",
            Self::InvalidTag => "invalid Windows observation tag",
            Self::InvalidWindow => "Windows observation capture window is reversed",
            Self::InvalidFileLength => "Windows observation EOF exceeds signed native length",
            Self::InconsistentDirectory => "Windows observation directory attribute disagrees",
        })
    }
}
impl std::error::Error for WindowsObservationError {}
