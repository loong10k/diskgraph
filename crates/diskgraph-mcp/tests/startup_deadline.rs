use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_mcp::{McpConfig, McpService};
use std::time::Instant;

#[test]
fn expired_local_and_remote_startup_never_create_databases() {
    for trusted_local in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let data = temporary.path().join("not-born");
        let result = McpService::open_with_host_until(
            McpConfig {
                data_dir: data.clone(),
                ..McpConfig::default()
            },
            None,
            trusted_local,
            Instant::now(),
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
        assert!(!data.exists());
    }
}

#[test]
fn live_remote_startup_does_not_bootstrap_local_policy() {
    let temporary = tempfile::tempdir().unwrap();
    let config = McpConfig {
        data_dir: temporary.path().join("data"),
        ..McpConfig::default()
    };
    let (service, recovery) = McpService::open_with_host_until(
        config.clone(),
        None,
        false,
        Instant::now() + std::time::Duration::from_secs(30),
    )
    .unwrap();
    assert!(recovery.is_none());
    let store =
        diskgraph_store::ControlStore::open(&config.data_dir.join("diskgraph-control.sqlite"))
            .unwrap();
    assert!(store.policy_state().unwrap().is_none());
    drop(service);
}
