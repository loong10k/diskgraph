//! 旧 FFI growth 的原始控制锁期限回归；来源：OpenSpec SC-06。

use crate::{native_growth, native_reply_deadline_tests};
use diskgraph_engine::{Engine, EngineConfig};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[test]
fn growth_initial_policy_capture_keeps_original_deadline_under_control_lock() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: dir.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let control = engine.control_store().unwrap();
    let worker_engine = Arc::clone(&engine);
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let locator = serde_json::to_string(&diskgraph_core::ResourceLocator::NativePath(
            "unused".to_owned(),
        ))
        .unwrap();
        let deadline = Instant::now() + Duration::from_millis(30);
        started_tx.send(()).unwrap();
        let result = native_growth::query(
            &worker_engine,
            ["unused-before", "unused-after"],
            &locator,
            deadline,
            || panic!("initial expiry must not reach response encoding"),
        );
        done_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    let result = done_rx.recv_timeout(Duration::from_millis(300));
    // 真实红灯也先释放控制锁并回收原线程，不遗弃被阻塞的工作。
    drop(control);
    worker.join().unwrap();
    assert!(
        matches!(result, Ok(Err(ref error)) if error.to_lowercase().contains("budget")),
        "growth initial policy capture waited outside the deadline: {result:?}"
    );
}

#[test]
fn growth_terminal_check_does_not_rebuild_policy_under_control_lock() {
    let (dir, engine, snapshot) = native_reply_deadline_tests::published_fixture();
    let locator = serde_json::to_string(&diskgraph_core::ResourceLocator::NativePath(
        dir.path()
            .join("root")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ))
    .unwrap();
    let engine = Arc::new(engine);
    let worker_engine = Arc::clone(&engine);
    let (encoded_tx, encoded_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = native_growth::query(
            &worker_engine,
            [&snapshot, &snapshot],
            &locator,
            Instant::now() + Duration::from_millis(500),
            || {
                encoded_tx.send(()).unwrap();
                continue_rx.recv().unwrap();
            },
        );
        done_tx.send(result).unwrap();
    });
    let reached = encoded_rx.recv_timeout(Duration::from_secs(2));
    if reached.is_err() {
        drop(continue_tx);
        worker.join().unwrap();
        panic!("fixture did not reach actual encoded response: {reached:?}");
    }
    let control = engine.control_store().unwrap();
    continue_tx.send(()).unwrap();
    let result = done_rx.recv_timeout(Duration::from_millis(900));
    drop(control);
    worker.join().unwrap();
    assert!(
        matches!(result, Ok(Err(ref error)) if error.to_lowercase().contains("budget")),
        "growth terminal policy reconstruction waited outside the deadline: {result:?}"
    );
}

#[test]
fn captured_growth_policy_preserves_zero_null_and_actual_revocation() {
    let (dir, engine, snapshot) = native_reply_deadline_tests::published_fixture();
    let root = dir.path().join("root").canonicalize().unwrap();
    let locator = serde_json::to_string(&diskgraph_core::ResourceLocator::NativePath(
        root.to_str().unwrap().to_owned(),
    ))
    .unwrap();
    let result = native_growth::query(
        &engine,
        [&snapshot, &snapshot],
        &locator,
        Instant::now() + Duration::from_secs(1),
        || {},
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["delta_bytes"], "0");
    let missing = serde_json::to_string(&diskgraph_core::ResourceLocator::NativePath(
        root.join("missing").to_str().unwrap().to_owned(),
    ))
    .unwrap();
    let result = native_growth::query(
        &engine,
        [&snapshot, &snapshot],
        &missing,
        Instant::now() + Duration::from_secs(1),
        || {},
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(reply["ok"], true);
    assert!(reply["data"].is_null());
    // 在实际数据编码后撤销数据库策略，捕获能力必须不能让响应绕过末检。
    let result = native_growth::query(
        &engine,
        [&snapshot, &snapshot],
        &locator,
        Instant::now() + Duration::from_secs(1),
        || engine.control_store().unwrap().revoke_policy().unwrap(),
    );
    assert!(
        matches!(result, Err(ref error) if error.contains("permission_denied")),
        "captured growth capability bypassed actual withdrawal: {result:?}"
    );
}

#[test]
fn growth_does_not_decode_unrelated_principal_grants() {
    let (dir, engine, snapshot) = native_reply_deadline_tests::published_fixture();
    let locator = serde_json::to_string(&diskgraph_core::ResourceLocator::NativePath(
        dir.path()
            .join("root")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ))
    .unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    db.execute(
        "INSERT INTO grants (principal_id, permission, scope_id, policy_version)
         VALUES ('unrelated-principal', X'FF', 'unrelated-scope', 1)",
        [],
    )
    .unwrap();
    assert!(
        engine.policy_authorizer().is_err(),
        "unrelated row must actually be invalid"
    );
    let result = native_growth::query(
        &engine,
        [&snapshot, &snapshot],
        &locator,
        Instant::now() + Duration::from_secs(1),
        || {},
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["delta_bytes"], "0");
}
