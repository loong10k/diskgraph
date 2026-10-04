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
