use super::McpService;
use crate::McpConfig;
use diskgraph_core::BusinessError;
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use std::cell::Cell;

#[test]
fn bootstrap_terminal_budget_failure_refuses_service_and_preserves_policy() {
    let temporary = tempfile::tempdir().unwrap();
    let config = McpConfig {
        data_dir: temporary.path().join("data"),
        ..McpConfig::default()
    };
    let engine = Engine::open(EngineConfig {
        data_dir: config.data_dir.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let calls = Cell::new(0);
    // 第2检查点在真实本地授权引导之后；只注入预算失败，不模拟慢I/O。
    let result = McpService::finish_startup_checked(config.clone(), engine, true, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            Err(BusinessError::BudgetExceeded.into())
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert_eq!(calls.get(), 2);
    let store =
        diskgraph_store::ControlStore::open(&config.data_dir.join("diskgraph-control.sqlite"))
            .unwrap();
    assert!(store.policy_state().unwrap().is_some());
}
