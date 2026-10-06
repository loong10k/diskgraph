use super::McpTestRunner;
use diskgraph_engine::{Engine, EngineConfig, JobRunner};
use std::sync::Arc;

#[test]
fn idle_runner_is_actually_joined_before_source_owner_returns() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let source_path = source.path().to_path_buf();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: data.path().to_path_buf(),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let witness = Arc::downgrade(&engine);
    let runner = JobRunner::start(Arc::clone(&engine));
    assert!(
        Arc::strong_count(&engine) >= 3,
        "original runner worker was not created"
    );
    let owner = McpTestRunner::new(runner, source);
    drop(engine);
    assert!(source_path.exists());
    assert!(witness.upgrade().is_some());
    drop(owner);
    // 没有扫描job，仅证明原闲置线程已退出，不能替代活跃扫描的回收证据。
    assert!(
        witness.upgrade().is_none(),
        "runner retained Engine after test owner ended"
    );
    assert!(!source_path.exists());
}
