use super::ChildError;

/// 区分 OS 启动错误与调用方检查点原错，清理不改写取消或授权的业务类型。
/// 来源：原生 Rust diskgraph-engine 既有 probe 启动检查点提取。
#[derive(Debug)]
pub(crate) enum ChildSpawnError<E> {
    Operation(ChildError),
    Checkpoint {
        primary: E,
        cleanup: Option<ChildError>,
    },
}

impl<E> ChildSpawnError<E> {
    /// 保留借用检查点的原失败。参数：primary 为调用方错误；返回：未添加清理原因的启动失败。
    pub(crate) fn checkpoint(primary: E) -> Self {
        Self::Checkpoint {
            primary,
            cleanup: None,
        }
    }

    /// 合并已执行的清理。参数：result 为 OS 回收结果；返回：原失败与可选清理错误。
    pub(crate) fn with_cleanup(self, result: Result<(), ChildError>) -> Self {
        match self {
            Self::Operation(error) => Self::Operation(error.with_cleanup(result)),
            Self::Checkpoint { primary, cleanup } => Self::Checkpoint {
                primary,
                cleanup: match (cleanup, result) {
                    (None, Ok(())) => None,
                    (None, Err(error)) => Some(error),
                    (Some(error), result) => Some(error.with_cleanup(result)),
                },
            },
        }
    }
}

impl<E> From<ChildError> for ChildSpawnError<E> {
    fn from(error: ChildError) -> Self {
        Self::Operation(error)
    }
}
