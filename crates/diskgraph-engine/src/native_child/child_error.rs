use std::{fmt, io, sync::Arc};

/// OS 子进程层的能力、I/O 与清理错误，不持有业务预算或授权。
/// 来源：原生 Rust diskgraph-engine 的 Unix retained leader 与 Win32 Job 边界。
#[derive(Clone, Debug)]
pub(crate) enum ChildError {
    #[cfg(windows)]
    InvalidLimits,
    Unsupported(&'static str),
    #[cfg(any(windows, test))]
    Io(String),
    NativeIo {
        context: String,
        source: Arc<io::Error>,
    },
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
        Self::NativeIo {
            context: context.to_owned(),
            source: Arc::new(error),
        }
    }
    /// 参数：self 为原生失败或主错误与清理错误的组合。
    /// 返回：原主 I/O 对象的借用；纯诊断/不支持项没有原生错误码，不从文本推断。
    #[cfg(test)]
    pub(crate) fn native_io_error(&self) -> Option<&io::Error> {
        match self {
            Self::NativeIo { source, .. } => Some(source.as_ref()),
            Self::Cleanup { primary, .. } => primary.native_io_error(),
            _ => None,
        }
    }
}

impl fmt::Display for ChildError {
    /// 参数：output 为错误显示目标；返回：与既有平台诊断相同的格式化结果。
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(windows)]
            Self::InvalidLimits => write!(output, "invalid child command limits"),
            Self::Unsupported(reason) => write!(output, "child unsupported: {reason}"),
            #[cfg(any(windows, test))]
            Self::Io(message) => write!(output, "child I/O failed: {message}"),
            Self::NativeIo { context, source } => {
                write!(output, "child I/O failed: {context}: {source}")
            }
            Self::Cleanup { primary, cleanup } => {
                write!(output, "{primary}; cleanup also failed: {cleanup}")
            }
        }
    }
}

impl std::error::Error for ChildError {
    /// 参数：无；返回：原 io::Error 或组合错误的主原因，不暴露 Arc 包装本身。
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // 显式借原 io::Error；不能把 Arc 本身作为 source 暴露，否则复制后对象身份不同。
            Self::NativeIo { source, .. } => Some(source.as_ref()),
            Self::Cleanup { primary, .. } => Some(primary.as_ref()),
            _ => None,
        }
    }
}
