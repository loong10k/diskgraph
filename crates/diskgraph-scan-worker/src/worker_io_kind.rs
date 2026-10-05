use serde::{Deserialize, Serialize};
use std::io::ErrorKind;

/// 稳定的 helper IO 错误类别；来源：Rust std::io::ErrorKind，未知原生扩展保留 Other 及 OS 码。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerIoKind {
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
    pub fn from_native(kind: ErrorKind) -> Self {
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

    /// 恢复真实 wire 类别，不把消息文本或 OS 数字解释成授权结果。
    /// 参数：self 为已通过闭合 variant 解码的类别。
    /// 返回：对应标准库类别；Other 仍为 Other，原 OS 码由失败记录独立保留。
    pub fn into_native(self) -> ErrorKind {
        match self {
            Self::NotFound => ErrorKind::NotFound,
            Self::PermissionDenied => ErrorKind::PermissionDenied,
            Self::AlreadyExists => ErrorKind::AlreadyExists,
            Self::InvalidInput => ErrorKind::InvalidInput,
            Self::InvalidData => ErrorKind::InvalidData,
            Self::Interrupted => ErrorKind::Interrupted,
            Self::UnexpectedEof => ErrorKind::UnexpectedEof,
            Self::BrokenPipe => ErrorKind::BrokenPipe,
            Self::TimedOut => ErrorKind::TimedOut,
            Self::WouldBlock => ErrorKind::WouldBlock,
            Self::Unsupported => ErrorKind::Unsupported,
            Self::OutOfMemory => ErrorKind::OutOfMemory,
            Self::Other => ErrorKind::Other,
        }
    }
}
