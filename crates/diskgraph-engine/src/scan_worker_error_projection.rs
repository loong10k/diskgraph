use crate::EngineError;
use crate::native_child::ChildError;
#[cfg(any(target_os = "linux", windows))]
use crate::native_child::ChildSpawnError;
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_remote_error::ScanWorkerRemoteError;
use diskgraph_core::BusinessError;
use diskgraph_scan_worker::{ExecutionFailure, ProtocolBudgetError};
use std::io;

/// 新helper父链只投影固定类型，原检查点错误与原I/O来源保持，不解析错误文本。
/// 来源：原生 Rust PF-06 与既有EngineError::WithCleanup，不新增授权或成功状态。
pub(super) struct ScanWorkerErrorProjection;

impl ScanWorkerErrorProjection {
    /// 参数：failure 为已通过闭合解码、Hello 顺序及预算字段组合校验的 helper 失败。
    /// 返回：明确输出预算代码对应原业务预算类型；其他失败保留原 IO 类别/OS 码/消息。
    /// 本投影不代替 driver 的完整协议、双 EOF 与原进程正常退出许可。
    pub(super) fn remote(failure: ExecutionFailure) -> EngineError {
        if failure.code() == "output_budget" {
            BusinessError::BudgetExceeded.into()
        } else {
            ScanWorkerRemoteError::into_io(failure).into()
        }
    }

    /// 参数：error为原Child类型；返回：稳定能力类别或拥有原native来源的I/O，并保清理链。
    pub(super) fn child(error: ChildError) -> EngineError {
        match error {
            ChildError::Unsupported(_) => BusinessError::Unsupported.into(),
            ChildError::Cleanup { primary, cleanup } => {
                Self::cleanup(Self::child(*primary), Some(*cleanup))
            }
            #[cfg(windows)]
            ChildError::InvalidLimits => BusinessError::InvalidArgument.into(),
            #[cfg(any(windows, test))]
            ChildError::Io(message) => io::Error::other(message).into(),
            ChildError::NativeIo { context, source } => match std::sync::Arc::try_unwrap(source) {
                Ok(source) => source.into(),
                Err(source) => {
                    io::Error::new(source.kind(), ChildError::NativeIo { context, source }).into()
                }
            },
        }
    }

    /// 参数：primary为原主错，cleanup是已发生的实际清理结果；返回：原类别及独立清理来源。
    pub(super) fn cleanup(primary: EngineError, cleanup: Option<ChildError>) -> EngineError {
        match cleanup {
            Some(error) => EngineError::WithCleanup {
                primary: Box::new(primary),
                cleanup: io::Error::other(error),
            },
            None => primary,
        }
    }

    /// 参数：error为原启动结果；返回：原检查点优先、原I/O与cleanup独立的引擎错误。
    #[cfg(any(target_os = "linux", windows))]
    pub(super) fn launch(error: ChildSpawnError<EngineError>) -> EngineError {
        match error {
            ChildSpawnError::Operation(error) => Self::child(error),
            ChildSpawnError::Checkpoint { primary, cleanup } => Self::cleanup(primary, cleanup),
        }
    }

    /// 参数：failure为原父驱动失败，project只消费其原检查点；返回：原业务类型及实际cleanup。
    pub(super) fn driver<E>(
        failure: ScanWorkerFailure<E>,
        project: impl FnOnce(E) -> EngineError,
    ) -> EngineError {
        match failure {
            ScanWorkerFailure::Checkpoint { primary, cleanup } => {
                Self::cleanup(project(primary), cleanup)
            }
            ScanWorkerFailure::Protocol { source, cleanup } => {
                // 只有本地协议实际额度检查产生的类型可归预算；不读取诊断文本或泛化 InvalidData。
                let primary = if source
                    .get_ref()
                    .is_some_and(|error| error.is::<ProtocolBudgetError>())
                {
                    BusinessError::BudgetExceeded.into()
                } else {
                    source.into()
                };
                Self::cleanup(primary, cleanup)
            }
            ScanWorkerFailure::Native { source, cleanup } => {
                Self::cleanup(Self::child(source), cleanup)
            }
            ScanWorkerFailure::OutputLimit { cleanup }
            | ScanWorkerFailure::Deadline { cleanup } => {
                Self::cleanup(BusinessError::BudgetExceeded.into(), cleanup)
            }
            ScanWorkerFailure::Cancelled { cleanup } => {
                Self::cleanup(BusinessError::Conflict.into(), cleanup)
            }
            ScanWorkerFailure::Exit { code, cleanup } => Self::cleanup(
                io::Error::other(format!(
                    "scan worker exited without a successful tree: {code:?}"
                ))
                .into(),
                cleanup,
            ),
            ScanWorkerFailure::Stopped { cleanup } => {
                Self::cleanup(io::Error::from(io::ErrorKind::Interrupted).into(), cleanup)
            }
        }
    }
}
