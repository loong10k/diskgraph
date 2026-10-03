use crate::{LocatorEncoding, LocatorKind, QualifiedLocatorError};
use std::path::{Path, PathBuf};

/// 具有明确来源编码的持久资源定位；来源：DiskGraph 原生 Rust D30，无 Java 对应对象。
/// 公开字段供显式存储映射，进入原生路径前必须重新校验；display 仅用于展示。
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct QualifiedLocator {
    /// 资源种类。
    pub kind: LocatorKind,
    /// 原始身份的明确编码。
    pub encoding: LocatorEncoding,
    /// 数据库 BLOB 保存的原始字节。
    pub raw: Vec<u8>,
    /// 与原始身份一致的显示投影，不是访问目标。
    pub display: String,
}

impl QualifiedLocator {
    /// 捕获本机路径；参数：path 为真实原生路径；返回：明确编码的定位或校验错误。
    pub fn from_native_path(path: &Path) -> Result<Self, QualifiedLocatorError> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Self::from_parts(
                LocatorKind::NativePath,
                LocatorEncoding::UnixBytes,
                path.as_os_str().as_bytes().to_vec(),
                path.to_string_lossy().into_owned(),
            )
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self::from_parts(
                LocatorKind::NativePath,
                LocatorEncoding::WindowsUtf16Le,
                path.as_os_str()
                    .encode_wide()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
                path.to_string_lossy().into_owned(),
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(QualifiedLocatorError::UnsupportedPlatform)
        }
    }

    /// 捕获文档 URI；参数：uri 为不透明 URI 文本；返回：UTF-8 定位或校验错误。
    pub fn from_document_uri(uri: impl AsRef<str>) -> Result<Self, QualifiedLocatorError> {
        let uri = uri.as_ref();
        Self::from_parts(
            LocatorKind::DocumentUri,
            LocatorEncoding::Utf8Uri,
            uri.as_bytes().to_vec(),
            uri.to_owned(),
        )
    }

    /// 校验持久字段；参数：kind/encoding/raw/display 为完整记录；返回：定位或格式错误。
    /// 外平台原生编码可保真保存，是否可在本机使用由 to_native_path 单独判断。
    pub fn from_parts(
        kind: LocatorKind,
        encoding: LocatorEncoding,
        raw: Vec<u8>,
        display: String,
    ) -> Result<Self, QualifiedLocatorError> {
        let locator = Self {
            kind,
            encoding,
            raw,
            display,
        };
        locator.validate()?;
        Ok(locator)
    }

    /// 借用校验公开字段；参数：无；返回：类型、编码、原始字节和展示投影一致或具体错误。
    /// 校验不复制原始字节；外平台合法编码仍可保存，校验成功不是文件操作授权。
    pub fn validate(&self) -> Result<(), QualifiedLocatorError> {
        if !matches!(
            (self.kind, self.encoding),
            (
                LocatorKind::NativePath,
                LocatorEncoding::UnixBytes | LocatorEncoding::WindowsUtf16Le
            ) | (LocatorKind::DocumentUri, LocatorEncoding::Utf8Uri)
        ) {
            return Err(QualifiedLocatorError::KindEncodingMismatch);
        }
        if self.raw.is_empty() {
            return Err(QualifiedLocatorError::EmptyIdentity);
        }
        let matches_display = match self.encoding {
            LocatorEncoding::UnixBytes => {
                if self.raw.contains(&0) {
                    return Err(QualifiedLocatorError::EmbeddedNul);
                }
                // 分段借用合法 UTF-8，非法序列按 from_utf8_lossy 的规则映射替换字符。
                unix_display_matches(&self.raw, &self.display)
            }
            LocatorEncoding::WindowsUtf16Le => {
                if !self.raw.len().is_multiple_of(2) {
                    return Err(QualifiedLocatorError::InvalidUtf16Length);
                }
                let units = self
                    .raw
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
                if units.clone().any(|unit| unit == 0) {
                    return Err(QualifiedLocatorError::EmbeddedNul);
                }
                // 与 String::from_utf16_lossy 相同的替换语义，逐字符比较避免暂存准入前分配。
                char::decode_utf16(units)
                    .map(|decoded| decoded.unwrap_or(char::REPLACEMENT_CHARACTER))
                    .eq(self.display.chars())
            }
            LocatorEncoding::Utf8Uri => {
                if self.raw.contains(&0) {
                    return Err(QualifiedLocatorError::EmbeddedNul);
                }
                let uri = std::str::from_utf8(&self.raw)
                    .map_err(|_| QualifiedLocatorError::InvalidUtf8Uri)?;
                uri == self.display
            }
        };
        if !matches_display {
            return Err(QualifiedLocatorError::DisplayMismatch);
        }
        Ok(())
    }

    /// 借用验证本机路径编码；参数：无；返回：可解析或明确拒绝，不分配原生路径副本。
    /// 校验公开字段完整性与宿主编码；成功只说明可解析，不授予文件访问权限。
    pub fn validate_native_path(&self) -> Result<(), QualifiedLocatorError> {
        self.validate()?;
        if self.kind != LocatorKind::NativePath {
            return Err(QualifiedLocatorError::NotNative);
        }
        #[cfg(unix)]
        let host_encoding = LocatorEncoding::UnixBytes;
        #[cfg(windows)]
        let host_encoding = LocatorEncoding::WindowsUtf16Le;
        #[cfg(any(unix, windows))]
        {
            if self.encoding != host_encoding {
                return Err(QualifiedLocatorError::ForeignPlatformEncoding);
            }
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        Err(QualifiedLocatorError::UnsupportedPlatform)
    }

    /// 按当前宿主还原路径；参数：无；返回：原生路径或明确拒绝，禁止展示字段回退。
    pub fn to_native_path(&self) -> Result<PathBuf, QualifiedLocatorError> {
        self.validate_native_path()?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            Ok(std::ffi::OsString::from_vec(self.raw.clone()).into())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let units: Vec<u16> = self
                .raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Ok(std::ffi::OsString::from_wide(&units).into())
        }
        #[cfg(not(any(unix, windows)))]
        Err(QualifiedLocatorError::UnsupportedPlatform)
    }

    /// 获取资源类型；参数：无；返回：声明类型。
    pub fn kind(&self) -> LocatorKind {
        self.kind
    }
    /// 获取原始编码；参数：无；返回：声明编码。
    pub fn encoding(&self) -> LocatorEncoding {
        self.encoding
    }
    /// 借用原始 BLOB；参数：无；返回：无复制的原始字节切片。
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }
    /// 获取展示投影；参数：无；返回：仅供人类展示的文本。
    pub fn display(&self) -> &str {
        &self.display
    }
}

fn unix_display_matches(mut raw: &[u8], display: &str) -> bool {
    let mut shown = display.chars();
    while !raw.is_empty() {
        match std::str::from_utf8(raw) {
            Ok(valid) => return valid.chars().eq(shown),
            Err(error) => {
                let valid_length = error.valid_up_to();
                let Ok(valid) = std::str::from_utf8(&raw[..valid_length]) else {
                    return false;
                };
                if valid
                    .chars()
                    .any(|character| shown.next() != Some(character))
                    || shown.next() != Some(char::REPLACEMENT_CHARACTER)
                {
                    return false;
                }
                let Some(invalid_length) = error.error_len() else {
                    return shown.next().is_none();
                };
                raw = &raw[valid_length + invalid_length..];
            }
        }
    }
    shown.next().is_none()
}
