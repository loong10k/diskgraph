//! 以既有存储、I/O、锁中毒及业务错误返回稳定失败语义。

use crate::ContextualEngineError;
use diskgraph_core::BusinessError;
use diskgraph_store::StoreError;
use std::io;

/// 以既有存储、I/O、锁中毒及业务错误返回稳定失败语义。
/// 来源：原生 Rust diskgraph-engine::EngineError。
/// Engine failures: storage, IO, or a business error carrying its stable code.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("engine poisoned by a previous panic")]
    Poisoned,
    #[error("{0}")]
    Business(#[from] BusinessError),
}

impl EngineError {
    /// 给引擎错误附加命令目录上下文。
    /// 参数：context 为产生错误的目录/能力名称。
    /// 返回：拥有原错误的上下文包装。
    /// Attaches the catalog entry that produced this error.
    pub fn with_context(self, context: impl Into<String>) -> ContextualEngineError {
        ContextualEngineError {
            error: self,
            context: context.into(),
        }
    }
}
impl From<ContextualEngineError> for EngineError {
    fn from(contextual: ContextualEngineError) -> Self {
        contextual.error
    }
}
