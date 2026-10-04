use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// 原生无损根路径；来源：Rust OsStr Unix bytes / Windows wide API，不使用展示路径回退。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", content = "units", rename_all = "snake_case")]
pub enum NativePath {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

impl NativePath {
    /// 编码当前平台根路径。参数为原生 path；返回原始字节或 UTF-16 单元。
    /// 参数：path 为当前操作系统的原生路径。
    /// 返回：无损 Unix 字节或 Windows UTF-16 单元记录，不使用 lossy 显示。
    pub fn from_path(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Self::Unix(path.as_os_str().as_bytes().to_vec())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self::Windows(path.as_os_str().encode_wide().collect())
        }
    }

    /// 恢复当前平台路径。返回无损 PathBuf；跨平台编码明确拒绝。
    /// 参数：self 为当前平台的原生根路径编码。
    /// 返回：绝对且不含 NUL 的 PathBuf；跨平台、相对路径或 NUL 返回错误。
    pub fn to_path(&self) -> io::Result<PathBuf> {
        let path = match self {
            #[cfg(unix)]
            Self::Unix(bytes) if !bytes.contains(&0) => {
                use std::os::unix::ffi::OsStrExt;
                Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
            }
            #[cfg(windows)]
            Self::Windows(units) if !units.contains(&0) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(PathBuf::from(std::ffi::OsString::from_wide(units)))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "foreign path encoding or NUL",
            )),
        }?;
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "root must be absolute",
            ));
        }
        Ok(path)
    }
}
