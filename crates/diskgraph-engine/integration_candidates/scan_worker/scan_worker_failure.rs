use crate::native_child::{ChildError, ChildSpawnError};
use std::io;
use std::time::Instant;

/// 父端执行的原检查点、协议与原生失败，不把清理失败覆盖成成功。
/// 来源：PF-06 原生 Child owner 与执行 v2 父驱动合同；E 不要求 Clone 或 Display。
#[derive(Debug)]
pub(crate) enum ScanWorkerFailure<E> {
    Checkpoint {
        primary: E,
        cleanup: Option<ChildError>,
    },
    Protocol {
        source: io::Error,
        cleanup: Option<ChildError>,
    },
    Native {
        source: ChildError,
        cleanup: Option<ChildError>,
    },
    OutputLimit {
        cleanup: Option<ChildError>,
    },
    Deadline {
        cleanup: Option<ChildError>,
    },
    Cancelled {
        cleanup: Option<ChildError>,
    },
    Exit {
        code: Option<i32>,
        cleanup: Option<ChildError>,
    },
    Stopped {
        cleanup: Option<ChildError>,
    },
}

impl<E> ScanWorkerFailure<E> {
    /// 参数：deadline 为原绝对期限，checkpoint 为原权限／fence／取消检查。
    /// 返回：原检查点错误优先保留，否则只检查原 Instant，不创建新的时间额度。
    pub(super) fn check(
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<(), Self> {
        checkpoint().map_err(|primary| Self::Checkpoint {
            primary,
            cleanup: None,
        })?;
        if Instant::now() >= deadline {
            return Err(Self::Deadline { cleanup: None });
        }
        Ok(())
    }

    /// 参数：result 为已经实际执行的本 Child 清理结果。
    /// 返回：原失败及独立清理失败；已有清理原因按实际先后合并，不丢原 E。
    pub(super) fn with_cleanup(mut self, result: Result<(), ChildError>) -> Self {
        if let Err(error) = result {
            let cleanup = match &mut self {
                Self::Checkpoint { cleanup, .. }
                | Self::Protocol { cleanup, .. }
                | Self::Native { cleanup, .. }
                | Self::OutputLimit { cleanup }
                | Self::Deadline { cleanup }
                | Self::Cancelled { cleanup }
                | Self::Exit { cleanup, .. }
                | Self::Stopped { cleanup } => cleanup,
            };
            *cleanup = Some(match cleanup.take() {
                Some(primary) => primary.with_cleanup(Err(error)),
                None => error,
            });
        }
        self
    }

    /// 参数：error 来自同一 Child 的正常退出检查，内部检查点也持有原 E。
    /// 返回：原原生错误或完整检查点失败，不按文本归一或克隆停止原因。
    pub(super) fn from_child(error: ChildSpawnError<Self>) -> Self {
        match error {
            ChildSpawnError::Operation(source) => Self::Native {
                source,
                cleanup: None,
            },
            ChildSpawnError::Checkpoint { primary, cleanup } => {
                primary.with_cleanup(cleanup.map_or(Ok(()), Err))
            }
        }
    }
}

impl<E> From<io::Error> for ScanWorkerFailure<E> {
    fn from(source: io::Error) -> Self {
        Self::Protocol {
            source,
            cleanup: None,
        }
    }
}

impl<E> From<ChildError> for ScanWorkerFailure<E> {
    fn from(source: ChildError) -> Self {
        Self::Native {
            source,
            cleanup: None,
        }
    }
}
