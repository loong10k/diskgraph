use serde::Serialize;
use std::io::ErrorKind;

/// 稳定的 helper IO 错误类别；来源：Rust std::io::ErrorKind，未知原生扩展保留 Other 及 OS 码。
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerIoKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    InvalidInput,
    InvalidData,
    Interrupted,
    UnexpectedEof,
    BrokenPipe,
    TimedOut,
    WouldBlock,
    Unsupported,
    OutOfMemory,
    Other,
}

impl WorkerIoKind {
    /// 参数：kind 为实际标准库 IO 错误类型。
    /// 返回：稳定线格式类别；未列出的平台类型保留 Other，不推断权限或成功。
    pub(crate) fn from_native(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::AlreadyExists => Self::AlreadyExists,
            ErrorKind::InvalidInput => Self::InvalidInput,
            ErrorKind::InvalidData => Self::InvalidData,
            ErrorKind::Interrupted => Self::Interrupted,
            ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            ErrorKind::BrokenPipe => Self::BrokenPipe,
            ErrorKind::TimedOut => Self::TimedOut,
            ErrorKind::WouldBlock => Self::WouldBlock,
            ErrorKind::Unsupported => Self::Unsupported,
            ErrorKind::OutOfMemory => Self::OutOfMemory,
            _ => Self::Other,
        }
    }
}
