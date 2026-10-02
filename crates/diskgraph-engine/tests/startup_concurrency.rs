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

#[test]
fn dropping_an_idle_runner_releases_its_engine() {
    let workspace = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let observer = std::sync::Arc::downgrade(&engine);
    let runner = diskgraph_engine::JobRunner::start(engine.clone());
    drop(runner);
    drop(engine);
    let deadline = Instant::now() + Duration::from_secs(2);
    while observer.strong_count() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        observer.strong_count(),
        0,
        "discarded runner kept its worker and SQLite connections alive"
    );
}

#[test]
fn dropping_a_runner_blocked_on_queue_read_prevents_a_new_claim() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), "data").unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: directory.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let principal = diskgraph_core::PrincipalId::new("fixture").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let blocked = engine.control_store().unwrap();
    let runner = diskgraph_engine::JobRunner::start(engine.clone());
    std::thread::sleep(Duration::from_millis(200));
    drop(runner);
    drop(blocked);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::sync::Arc::strong_count(&engine) > 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::sync::Arc::strong_count(&engine), 1);
    let status = engine.job_status(&job.job_id).unwrap();
    assert_eq!(status.state, diskgraph_store::JobState::Queued);
    assert_eq!(status.fencing_token, 0);
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}
