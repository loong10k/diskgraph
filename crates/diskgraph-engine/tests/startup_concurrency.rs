//! Existing read-only CLI sessions should not need the graph writer lock to start.

use std::time::{Duration, Instant};

use diskgraph_engine::{Engine, EngineConfig};

#[test]
fn opening_an_existing_engine_does_not_wait_for_the_graph_writer() {
    let workspace = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        data_dir: workspace.path().join("data"),
        ..EngineConfig::default()
    };
    drop(Engine::open(config.clone()).unwrap());
    let writer = rusqlite::Connection::open(config.data_dir.join("diskgraph.sqlite")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();

    let started = Instant::now();
    let reopened = Engine::open(config);
    writer.execute_batch("ROLLBACK").unwrap();
    assert!(
        reopened.is_ok(),
        "existing store open needs a writer: {:?}",
        reopened.err()
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "existing store open waited for a graph writer"
    );
}
