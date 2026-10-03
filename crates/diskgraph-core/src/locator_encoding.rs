use crate::QualifiedLocatorError;
use serde::{Deserialize, Serialize};

/// 可持久化的资源原始编码；来源：DiskGraph 原生 Rust D30，无 Java 对应对象。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocatorEncoding {
    /// Unix 路径原始字节，不要求 UTF-8。
    UnixBytes,
    /// Windows UTF-16 代码单元的小端字节，允许未配对代理项。
    WindowsUtf16Le,
    /// 文档提供方 URI 的 UTF-8 字节。
    Utf8Uri,
}

impl LocatorEncoding {
    /// 返回稳定数据库标签；参数：无；返回：当前编码的 snake_case 标签。
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::UnixBytes => "unix_bytes",
            Self::WindowsUtf16Le => "windows_utf16_le",
            Self::Utf8Uri => "utf8_uri",
        }
    }

    /// 解析明确编码标签；参数：label 为数据库文本；返回：编码或未知标签错误。
    pub fn parse(label: &str) -> Result<Self, QualifiedLocatorError> {
        match label {
            "unix_bytes" => Ok(Self::UnixBytes),
            "windows_utf16_le" => Ok(Self::WindowsUtf16Le),
            "utf8_uri" => Ok(Self::Utf8Uri),
            _ => Err(QualifiedLocatorError::UnknownEncoding(label.into())),
        }
    }
}
