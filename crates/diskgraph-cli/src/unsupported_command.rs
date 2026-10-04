//! CLI unsupported_command 的真实职责实现。
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;

/// 保留 unsupported 的原生业务职责与错误语义。来源：DiskGraph CLI main::unsupported。
/// 参数：与原入口的 unsupported 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn unsupported(catalog_id: &str, stage: &str) -> EngineError {
    eprintln!("{catalog_id} is not enabled in this read-only build (planned for {stage})");
    EngineError::Business(BusinessError::Unsupported)
}
