use std::io;

/// OS 子进程层的能力、I/O 与清理错误，不持有业务预算或授权。
/// 来源：原生 Rust diskgraph-engine 的 Unix retained leader 与 Win32 Job 边界。
#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum ChildError {
    #[cfg(windows)]
    #[error("invalid child command limits")]
    InvalidLimits,
    #[error("child unsupported: {0}")]
    Unsupported(&'static str),
    #[error("child I/O failed: {0}")]
    Io(String),
    #[error("{primary}; cleanup also failed: {cleanup}")]
    Cleanup {
        primary: Box<ChildError>,
        cleanup: Box<ChildError>,
    },
}

impl ChildError {
    /// 合并原 OS 错误与实际清理结果。参数：cleanup 为回收结果；返回：原错或双重原因。
    pub(crate) fn with_cleanup(self, cleanup: Result<(), Self>) -> Self {
        match cleanup {
            Ok(()) => self,
            Err(cleanup) => Self::Cleanup {
                primary: Box::new(self),
                cleanup: Box::new(cleanup),
            },
        }
    }

    /// 保留 OS 错误阶段。参数：context 为原操作名，error 为系统错误；返回：I/O 诊断。
    pub(crate) fn io(context: &str, error: io::Error) -> Self {
        Self::Io(format!("{context}: {error}"))
    }
}
