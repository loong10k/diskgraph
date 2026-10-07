//! 以既有存储、I/O、锁中毒及业务错误返回稳定失败语义。

use crate::ContextualEngineError;
use diskgraph_core::BusinessError;
use diskgraph_store::StoreError;
use std::{fmt, io};

/// 以既有存储、I/O、锁中毒及业务错误返回稳定失败语义。
/// 来源：原生 Rust diskgraph-engine::EngineError。
/// Engine failures: storage, IO, or a business error carrying its stable code.
#[derive(Debug)]
pub enum EngineError {
    Store(StoreError),
    Io(io::Error),
    Poisoned,
    Business(BusinessError),
    /// 保留原主原因及已经发生的清理 I/O，不改变业务分类或扩大操作授权。
    WithCleanup {
        primary: Box<EngineError>,
        cleanup: io::Error,
    },
}

impl EngineError {
    /// 借用最内层原主错误，用于稳定分类而不丢失外层清理诊断。
    /// 参数：无；self 为原错误或嵌套清理包装。
    /// 返回：原主错误对象的借用；迭代遍历，不克隆、格式化或递归解析。
    pub fn primary(&self) -> &Self {
        let mut error = self;
        while let Self::WithCleanup { primary, .. } = error {
            error = primary.as_ref();
        }
        error
    }

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

impl fmt::Display for EngineError {
    /// 参数：output 为格式化目标；self 为原错误或清理包装。
    /// 返回：原诊断，或同时包含原主错误和实际清理失败的格式化结果。
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => fmt::Display::fmt(error, output),
            Self::Io(error) => fmt::Display::fmt(error, output),
            Self::Poisoned => output.write_str("engine poisoned by a previous panic"),
            Self::Business(error) => fmt::Display::fmt(error, output),
            Self::WithCleanup { primary, cleanup } => {
                write!(output, "{primary}; cleanup also failed: {cleanup}")
            }
        }
    }
}

impl std::error::Error for EngineError {
    /// 参数：无；self 为拥有原对象的错误。
    /// 返回：旧透明变体的原 source；清理包装直接借用原主 EngineError。
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            // 保持旧 transparent 变体委托；仅新包装直接借用原 EngineError。
            Self::Store(error) => std::error::Error::source(error),
            Self::Io(error) => std::error::Error::source(error),
            Self::Poisoned => None,
            Self::Business(error) => Some(error),
            Self::WithCleanup { primary, .. } => Some(primary.as_ref()),
        }
    }
}

impl From<StoreError> for EngineError {
    /// 参数：error 为原存储错误；返回：原 Store 变体，不附加清理诊断。
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<io::Error> for EngineError {
    /// 参数：error 为原 I/O 对象；返回：原 Io 变体，保留 errno 和 source。
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BusinessError> for EngineError {
    /// 参数：error 为原业务类别；返回：原 Business 变体，保持稳定错误码。
    fn from(error: BusinessError) -> Self {
        Self::Business(error)
    }
}

impl From<crate::recovery_slot::SlotError> for EngineError {
    /// 参数：error为原监督准入失败；返回：稳定分类或保留原I/O，绝不按诊断文本猜测恢复状态。
    fn from(error: crate::recovery_slot::SlotError) -> Self {
        use crate::recovery_slot::SlotError;
        match error {
            SlotError::Busy => BusinessError::ResourceExhausted.into(),
            SlotError::Unconfirmed => BusinessError::RecoveryUnconfirmed.into(),
            SlotError::InvalidRecord => BusinessError::NeedsAttention.into(),
            SlotError::Unsupported => BusinessError::Unsupported.into(),
            SlotError::Deadline => BusinessError::BudgetExceeded.into(),
            SlotError::Io(error) => Self::Io(error),
        }
    }
}

impl From<ContextualEngineError> for EngineError {
    /// 参数：contextual 为旧上下文包装；返回：其中拥有的原 EngineError。
    fn from(contextual: ContextualEngineError) -> Self {
        contextual.error
    }
}
