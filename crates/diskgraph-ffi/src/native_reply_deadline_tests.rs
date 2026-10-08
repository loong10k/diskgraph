//! 原生查询初始授权不能在请求期限外等待共享控制锁。
use crate::native_reply;
use diskgraph_engine::{Engine, EngineConfig};
use std::sync::{Arc, atomic::AtomicBool, mpsc};
use std::time::{Duration, Instant};

#[test]
fn native_query_policy_capture_does_not_wait_for_an_unbounded_control_owner() {
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
        let deadline = Instant::now() + Duration::from_millis(30);
        started_tx.send(()).unwrap();
        let result = native_reply::query(
            &worker_engine,
            "unused-snapshot",
            deadline,
            Arc::new(AtomicBool::new(false)),
            |_, _| panic!("expired authorization must not read data"),
            || {},
        );
        done_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    let result = done_rx.recv_timeout(Duration::from_millis(300));
    // 红灯路径也必须释放锁并 join，不能遗弃等待线程。
    drop(control);
    worker.join().unwrap();
    assert!(
        matches!(result, Ok(Err(ref error)) if error.to_lowercase().contains("budget")),
        "policy capture exceeded original deadline: {result:?}"
    );
}

fn published_fixture() -> (tempfile::TempDir, Engine, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"fixture").unwrap();
    let graph_path = dir.path().join("graph.sqlite");
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        graph_database_path: Some(graph_path.clone()),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = crate::local_principal().unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    // 仅建立真实图库夹具，不替代受管 worker 的原生扫描资格验收。
    let graph = diskgraph_disktree::scan_native(&root, Default::default()).unwrap();
    let mut store = diskgraph_store::SqliteSnapshotStore::open(&graph_path).unwrap();
    store.append_staging_nodes("fixture", &graph.nodes).unwrap();
    store
        .publish_revision_owned(
            "fixture",
            &graph,
            "fixture-revision",
            1,
            Some((engine.server_id().unwrap().as_str(), scope.as_str())),
        )
        .unwrap();
    (dir, engine, graph.snapshot.id)
}

#[test]
fn captured_policy_does_not_bypass_live_revocation_during_read() {
    let (_dir, engine, snapshot) = published_fixture();
    let answer = native_reply::query(
        &engine,
        &snapshot,
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
        |_, _| {
            engine.control_store().unwrap().revoke_policy().unwrap();
            Ok(serde_json::json!({"sensitive": "must not escape"}))
        },
        || {},
    );
    assert!(
        answer.is_err(),
        "captured capability bypassed live revocation: {answer:?}"
    );
}

#[test]
fn captured_policy_still_allows_a_normal_authorized_query() {
    let (_dir, engine, snapshot) = published_fixture();
    let answer = native_reply::query(
        &engine,
        &snapshot,
        Instant::now() + Duration::from_secs(1),
        Arc::new(AtomicBool::new(false)),
        |store, _| {
            assert!(store.node(&snapshot, 1).unwrap().is_some());
            Ok(serde_json::json!({"node": 1}))
        },
        || {},
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["data"]["node"], 1);
}

#[test]
fn native_query_terminal_check_does_not_rebuild_policy_under_a_blocked_control_owner() {
    let (_dir, engine, snapshot) = published_fixture();
    let engine = Arc::new(engine);
    let worker_engine = Arc::clone(&engine);
    let (read_tx, read_rx) = mpsc::channel();
    let (continue_tx, continue_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let answer = native_reply::query(
            &worker_engine,
            &snapshot,
            Instant::now() + Duration::from_millis(500),
            Arc::new(AtomicBool::new(false)),
            |_, _| {
                read_tx.send(()).unwrap();
                continue_rx.recv().unwrap();
                Ok(serde_json::json!({"node": 1}))
            },
            || {},
        );
        done_tx.send(answer).unwrap();
    });
    let reached_read = read_rx.recv_timeout(Duration::from_secs(2));
    if reached_read.is_err() {
        drop(continue_tx);
        worker.join().unwrap();
        panic!("fixture did not reach the actual read phase: {reached_read:?}");
    }
    let control = engine.control_store().unwrap();
    continue_tx.send(()).unwrap();
    let result = done_rx.recv_timeout(Duration::from_millis(900));
    drop(control);
    worker.join().unwrap();
    assert!(
        matches!(result, Ok(Err(ref error)) if error.to_lowercase().contains("budget")),
        "terminal policy reconstruction waited outside the original request: {result:?}"
    );
}
