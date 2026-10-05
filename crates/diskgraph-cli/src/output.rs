//! CLI output 的真实职责实现。
use diskgraph_core::{BusinessError, Envelope};
use diskgraph_engine::{Engine, EngineError};

/// 保留 engine_business 的原生业务职责与错误语义。来源：DiskGraph CLI main::engine_business。
/// 参数：与原入口的 engine_business 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn engine_business(error: &EngineError) -> BusinessError {
    match error.primary() {
        EngineError::Business(business) => *business,
        EngineError::Store(store) => match store {
            diskgraph_store::StoreError::SnapshotNotFound(_)
            | diskgraph_store::StoreError::ScopeNotFound(_)
            | diskgraph_store::StoreError::JobNotFound(_)
            | diskgraph_store::StoreError::RevisionNotFound(_) => BusinessError::NotFound,
            diskgraph_store::StoreError::Conflict(_) | diskgraph_store::StoreError::StaleOwner => {
                BusinessError::Conflict
            }
            diskgraph_store::StoreError::RetentionViolation(_) => BusinessError::Conflict,
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

/// 保留 envelope_line 的原生业务职责与错误语义。来源：DiskGraph CLI main::envelope_line。
/// 参数：与原入口的 envelope_line 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn envelope_line(
    engine: &Engine,
    result: Result<serde_json::Value, EngineError>,
) -> String {
    match result {
        Ok(data) => {
            let envelope = Envelope::ok(data).with_ids(engine.server_id().ok(), None, None);
            serde_json::to_string(&envelope).unwrap_or_else(|_| {
                serde_json::to_string(&Envelope::failure(BusinessError::InternalError, "encode"))
                    .unwrap()
            })
        }
        Err(error) => {
            let business = engine_business(&error);
            serde_json::to_string(&Envelope::failure(business, error.to_string())).unwrap()
        }
    }
}
