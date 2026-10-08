use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::BusinessError;
use std::cell::Cell;

#[test]
fn exhausted_check_after_graph_open_prevents_control_database_birth() {
    let temporary = tempfile::tempdir().unwrap();
    let data = temporary.path().join("data");
    let checks = Cell::new(0);
    // 检查点4位于真实图库初始化之后；确定性注入预算耗尽，不依赖磁盘速度。
    let result = Engine::open_checked(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        &|| {
            checks.set(checks.get() + 1);
            if checks.get() == 4 {
                Err(BusinessError::BudgetExceeded.into())
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert_eq!(checks.get(), 4);
    assert!(data.join("diskgraph.sqlite").is_file());
    assert!(!data.join("diskgraph-control.sqlite").exists());
}
