use crate::NativePath;
use serde::Deserialize;
use std::{io, path::PathBuf};

/// v2 严格原生根路径；来源：Rust OsStr bytes/UTF-16，保留旧 NativePath 的公开 serde 兼容形状。
#[derive(Deserialize)]
#[serde(
    tag = "platform",
    content = "units",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum WorkerPath {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

impl WorkerPath {
    /// 参数：self 为闭合请求中的原始平台单元。
    /// 返回：当前平台无损绝对路径；外来平台、NUL、相对路径明确拒绝。
    pub(crate) fn into_path(self) -> io::Result<PathBuf> {
        match self {
            Self::Unix(bytes) => NativePath::Unix(bytes).to_path(),
            Self::Windows(units) => NativePath::Windows(units).to_path(),
        }
    }
}
