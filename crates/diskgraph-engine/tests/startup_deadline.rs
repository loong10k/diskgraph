use diskgraph_core::BusinessError;
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use std::time::{Duration, Instant};

#[test]
fn expired_startup_refuses_before_data_directory_birth() {
    let temporary = tempfile::tempdir().unwrap();
    let data = temporary.path().join("not-born");
    let result = Engine::open_until(
        EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        },
        Instant::now(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(!data.exists());
}

#[test]
fn live_startup_preserves_existing_server_identity() {
    let temporary = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        data_dir: temporary.path().join("data"),
        ..EngineConfig::default()
    };
    let first =
        Engine::open_until(config.clone(), Instant::now() + Duration::from_secs(30)).unwrap();
    let identity = first.server_id().unwrap();
    drop(first);
    let second = Engine::open_until(config, Instant::now() + Duration::from_secs(30)).unwrap();
    assert_eq!(identity, second.server_id().unwrap());
}
