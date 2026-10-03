/// 编码限定定位的校验和宿主转换错误；来源：DiskGraph 原生 Rust D30，无 Java 对应对象。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QualifiedLocatorError {
    /// 未识别的持久编码标签。
    UnknownEncoding(String),
    /// 资源类型与字节编码不一致。
    KindEncodingMismatch,
    /// 原始身份为空。
    EmptyIdentity,
    /// 路径或 URI 含有 NUL。
    EmbeddedNul,
    /// UTF-16LE 字节长度不是偶数。
    InvalidUtf16Length,
    /// URI 原始字段不是有效 UTF-8。
    InvalidUtf8Uri,
    /// 展示内容与原始身份的显示投影不一致。
    DisplayMismatch,
    /// 原生路径的编码不属于当前宿主。
    ForeignPlatformEncoding,
    /// 文档 URI 不能转换为原生路径。
    NotNative,
    /// 当前平台没有定义可保真的原生路径编码。
    UnsupportedPlatform,
}

impl std::fmt::Display for QualifiedLocatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownEncoding(label) => write!(formatter, "unknown locator encoding: {label}"),
            Self::KindEncodingMismatch => formatter.write_str("locator kind and encoding disagree"),
            Self::EmptyIdentity => formatter.write_str("locator identity is empty"),
            Self::EmbeddedNul => formatter.write_str("locator identity contains NUL"),
            Self::InvalidUtf16Length => formatter.write_str("locator UTF-16LE byte length is odd"),
            Self::InvalidUtf8Uri => formatter.write_str("locator URI is not valid UTF-8"),
            Self::DisplayMismatch => {
                formatter.write_str("locator display differs from raw identity")
            }
            Self::ForeignPlatformEncoding => {
                formatter.write_str("locator native encoding belongs to another platform")
            }
            Self::NotNative => formatter.write_str("document URI is not a native path"),
            Self::UnsupportedPlatform => {
                formatter.write_str("native locator encoding is unsupported on this platform")
            }
        }
    }
}
impl std::error::Error for QualifiedLocatorError {}
