use crate::NativePath;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

/// v2 严格原生根路径；来源：Rust OsStr bytes/UTF-16，保留旧 NativePath 的公开 serde 兼容形状。
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "platform",
    content = "units",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkerPath {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

impl WorkerPath {
    /// 从父端已验证的真实根路径构造同一 v2 记录，不探测文件或改变挂载语义。
    /// 参数：path 为当前平台的绝对原生路径。
    /// 返回：保留原始 bytes/UTF-16 的路径；相对路径或 NUL 明确拒绝。
    pub fn from_path(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "relative worker root",
            ));
        }
        match NativePath::from_path(path) {
            NativePath::Unix(bytes) if !bytes.contains(&0) => Ok(Self::Unix(bytes)),
            NativePath::Windows(units) if !units.contains(&0) => Ok(Self::Windows(units)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NUL worker root",
            )),
        }
    }

    /// 参数：self 为闭合请求中的原始平台单元。
    /// 返回：当前平台无损绝对路径；外来平台、NUL、相对路径明确拒绝。
    pub fn into_path(self) -> io::Result<PathBuf> {
        match self {
            Self::Unix(bytes) => NativePath::Unix(bytes).to_path(),
            Self::Windows(units) => NativePath::Windows(units).to_path(),
        }
    }
}
