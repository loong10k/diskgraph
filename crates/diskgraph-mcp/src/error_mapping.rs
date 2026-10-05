//! 保持引擎错误到稳定业务错误码的原始映射。来源：原生 Rust MCP 服务。
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;

/// 保持引擎错误到稳定业务错误码的原始映射。
/// 参数：error 为原始引擎错误。返回：对应业务错误码。
pub(crate) fn business_of(error: &EngineError) -> BusinessError {
    match error.primary() {
        EngineError::Business(business) => *business,
        EngineError::Store(store) => match store {
            diskgraph_store::StoreError::SnapshotNotFound(_)
            | diskgraph_store::StoreError::ScopeNotFound(_)
            | diskgraph_store::StoreError::JobNotFound(_)
            | diskgraph_store::StoreError::RevisionNotFound(_) => BusinessError::NotFound,
            diskgraph_store::StoreError::Conflict(_)
            | diskgraph_store::StoreError::StaleOwner
            | diskgraph_store::StoreError::RetentionViolation(_) => BusinessError::Conflict,
            diskgraph_store::StoreError::BudgetExceeded => BusinessError::BudgetExceeded,
            error if error.is_interrupted() => BusinessError::BudgetExceeded,
            _ => BusinessError::InternalError,
        },
        // primary() 已剥离清理包装；显式覆盖完整枚举，不用 panic 表示不可能分支。
        EngineError::Io(_) | EngineError::Poisoned | EngineError::WithCleanup { .. } => {
            BusinessError::InternalError
        }
    }
}
