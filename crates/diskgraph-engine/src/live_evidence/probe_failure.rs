use std::io;

/// 区分资源中止、平台能力与命令 I/O 错误，禁止降级为成功观察。
/// 来源：原生 Rust diskgraph-engine::live_evidence::ProbeFailure。
#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum ProbeFailure {
    #[error("probe deadline exceeded")]
    Deadline,
    #[error("probe cancelled")]
    Cancelled,
    #[error("probe cumulative output byte limit exceeded")]
    OutputLimit,
    #[error("probe cumulative input or private resource limit exceeded")]
    ResourceLimit,
    #[error("indexed Git directory identity changed")]
    IdentityChanged,
    #[error("invalid probe limits")]
    InvalidLimits,
    #[error("probe terminated without a supported normal exit status: {0:?}")]
    AbnormalExit(Option<i32>),
    #[error("probe unsupported: {0}")]
    Unsupported(&'static str),
    #[error("probe I/O failed: {0}")]
    Io(String),
    #[error("{primary}; cleanup also failed: {cleanup}")]
    Cleanup {
        primary: Box<ProbeFailure>,
        cleanup: Box<ProbeFailure>,
    },
}

impl ProbeFailure {
    /// 保留原失败与显式清理失败，禁止将资源泄漏诊断隐藏在 Drop 中。
    /// 参数：cleanup 为已经执行的清理结果。
    /// 返回：原失败或同时包含两项原因的失败。
    pub(super) fn with_cleanup(self, cleanup: Result<(), Self>) -> Self {
        match cleanup {
            Ok(()) => self,
            Err(cleanup) => Self::Cleanup {
                primary: Box::new(self),
                cleanup: Box::new(cleanup),
            },
        }
    }
    /// 保留失败阶段和操作系统错误，避免把失败解释成空结果。
    /// 参数：context 为失败阶段，error 为原始 I/O 错误。
    /// 返回：可跨平台传播并锁存的错误诊断。
    pub(super) fn io(context: &str, error: io::Error) -> Self {
        Self::Io(format!("{context}: {error}"))
    }
}

// OS 层不持业务错误；原探针边界逐项恢复既有类型、文本与清理原因。
impl From<crate::native_child::ChildError> for ProbeFailure {
    fn from(error: crate::native_child::ChildError) -> Self {
        use crate::native_child::ChildError;
        match error {
            #[cfg(windows)]
            ChildError::InvalidLimits => Self::InvalidLimits,
            ChildError::Unsupported(reason) => Self::Unsupported(reason),
            #[cfg(any(windows, test))]
            ChildError::Io(reason) => Self::Io(reason),
            ChildError::NativeIo { context, source } => Self::Io(format!("{context}: {source}")),
            ChildError::Cleanup { primary, cleanup } => Self::Cleanup {
                primary: Box::new(Self::from(*primary)),
                cleanup: Box::new(Self::from(*cleanup)),
            },
        }
    }
}

impl From<crate::native_child::ChildSpawnError<ProbeFailure>> for ProbeFailure {
    fn from(error: crate::native_child::ChildSpawnError<Self>) -> Self {
        use crate::native_child::ChildSpawnError;
        match error {
            ChildSpawnError::Operation(error) => Self::from(error),
            ChildSpawnError::Checkpoint { primary, cleanup } => {
                primary.with_cleanup(cleanup.map_or(Ok(()), |error| Err(Self::from(error))))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProbeFailure;
    use crate::native_child::{ChildError, ChildSpawnError};

    #[test]
    fn original_probe_checkpoint_and_cleanup_errors_keep_their_types_and_text() {
        let source = ProbeFailure::Cancelled.with_cleanup(Err(ProbeFailure::Io("reap".into())));
        #[cfg(unix)]
        let error = ChildSpawnError::checkpoint(ProbeFailure::Cancelled)
            .with_cleanup(Err(ChildError::Io("reap".into())));
        // Windows由调用方持有原owner，并显式组合检查点与清理失败。
        #[cfg(not(unix))]
        let error = ChildSpawnError::Checkpoint {
            primary: ProbeFailure::Cancelled,
            cleanup: Some(ChildError::Io("reap".into())),
        };
        let moved = ProbeFailure::from(error);
        assert_eq!(moved.to_string(), source.to_string());
        assert!(matches!(moved, ProbeFailure::Cleanup { primary, cleanup }
            if matches!(*primary, ProbeFailure::Cancelled)
            && matches!(*cleanup, ProbeFailure::Io(ref value) if value == "reap")));
    }

    #[test]
    fn nested_os_cleanup_is_mapped_without_changing_the_original_probe_diagnostic() {
        let old = ProbeFailure::Io("read".into())
            .with_cleanup(Err(ProbeFailure::Io("terminate".into())))
            .with_cleanup(Err(ProbeFailure::Unsupported("lost owner")));
        let error = ChildError::Io("read".into())
            .with_cleanup(Err(ChildError::Io("terminate".into())))
            .with_cleanup(Err(ChildError::Unsupported("lost owner")));
        assert_eq!(ProbeFailure::from(error).to_string(), old.to_string());
    }
    #[test]
    fn native_io_projection_preserves_legacy_text_and_cancelled_primary() {
        let directory = tempfile::tempdir().unwrap();
        let original = std::fs::File::open(directory.path().join("missing")).unwrap_err();
        let expected = format!(
            "probe cancelled; cleanup also failed: probe I/O failed: open native fixture: {original}"
        );
        #[cfg(unix)]
        let error = ChildSpawnError::checkpoint(ProbeFailure::Cancelled)
            .with_cleanup(Err(ChildError::io("open native fixture", original)));
        #[cfg(not(unix))]
        let error = ChildSpawnError::Checkpoint {
            primary: ProbeFailure::Cancelled,
            cleanup: Some(ChildError::io("open native fixture", original)),
        };
        let mapped = ProbeFailure::from(error);
        assert_eq!(mapped.to_string(), expected);
        assert!(matches!(mapped, ProbeFailure::Cleanup { primary, cleanup }
            if matches!(*primary, ProbeFailure::Cancelled)
            && matches!(*cleanup, ProbeFailure::Io(_))));
    }
}
