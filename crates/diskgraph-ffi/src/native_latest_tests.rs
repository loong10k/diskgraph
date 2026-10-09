//! 最新快照查询的真实控制锁和编码后撤权回归，不使用伪数据库。
use crate::{native_latest, native_reply_deadline_tests};
use diskgraph_engine::{Engine, EngineConfig};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[test]
fn latest_policy_capture_keeps_original_control_lock_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        })
        .unwrap(),
    );
    let control = engine.control_store().unwrap();
    let worker_engine = Arc::clone(&engine);
    let root = dir.path().to_owned();
    let (start_tx, start_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(30);
        start_tx.send(()).unwrap();
        done_tx
            .send(native_latest::query(&worker_engine, &root, deadline, || {}))
            .unwrap();
    });
    start_rx.recv().unwrap();
    let result = done_rx.recv_timeout(Duration::from_millis(300));
    drop(control);
    worker.join().unwrap();
    assert!(
        matches!(result, Ok(Err(ref e)) if e.to_lowercase().contains("budget")),
        "latest waited outside original deadline: {result:?}"
    );
}

#[test]
fn latest_encoded_success_is_refused_after_real_policy_revocation() {
    let (dir, engine, _) = native_reply_deadline_tests::published_fixture();
    let root = dir.path().join("root").canonicalize().unwrap();
    let result = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || {
            engine.control_store().unwrap().revoke_policy().unwrap();
        },
    );
    assert!(
        matches!(result, Err(ref e) if e.contains("permission_denied")),
        "latest released data after actual withdrawal: {result:?}"
    );
}

#[test]
fn latest_reads_id_without_decoding_unused_snapshot_document() {
    let (dir, engine, snapshot) = native_reply_deadline_tests::published_fixture();
    let database = rusqlite::Connection::open(dir.path().join("graph.sqlite")).unwrap();
    database
        .execute(
            "UPDATE snapshots SET snapshot_json=?1 WHERE id=?2",
            rusqlite::params!["{broken unused document", &snapshot],
        )
        .unwrap();
    let root = dir.path().join("root").canonicalize().unwrap();
    let result = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || {},
    );
    assert!(
        result.is_ok(),
        "latest decoded unrelated snapshot document: {result:?}"
    );
    let reply: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(reply["data"]["snapshot_id"], snapshot);
}

#[test]
fn latest_refuses_oversized_snapshot_id_before_owning_the_result() {
    let (dir, engine, _) = native_reply_deadline_tests::published_fixture();
    let oversized = "s".repeat(2 * 1024 * 1024);
    let root = dir.path().join("root").canonicalize().unwrap();
    let principal = crate::local_principal().unwrap();
    let scope = engine
        .list_scopes(&principal, &engine.policy_authorizer().unwrap())
        .unwrap()
        .pop()
        .unwrap()
        .scope_id;
    // 使用当前真实发布接口保留外键和 writer 门禁，不绕过保护触发器构造缺失目标。
    let mut graph = diskgraph_disktree::scan_native(&root, Default::default()).unwrap();
    graph.snapshot.id = oversized;
    let mut store =
        diskgraph_store::SqliteSnapshotStore::open(&dir.path().join("graph.sqlite")).unwrap();
    store
        .append_staging_nodes("oversized", &graph.nodes)
        .unwrap();
    store
        .publish_revision_owned(
            "oversized",
            &graph,
            "oversized-revision",
            2,
            Some((engine.server_id().unwrap().as_str(), scope.as_str())),
        )
        .unwrap();
    let budget = diskgraph_core::QueryBudget::default();
    assert!(graph.snapshot.id.len() > budget.max_response_bytes);
    let mut reads =
        diskgraph_core::QueryReadBudget::new(budget, Instant::now() + Duration::from_secs(5))
            .unwrap();
    let direct = store.latest_snapshot_ids_with_budget(
        engine.server_id().unwrap().as_str(),
        scope.as_str(),
        &mut reads,
    );
    assert!(matches!(
        direct,
        Err(diskgraph_store::StoreError::BudgetExceeded)
    ));
    assert_eq!(
        reads.stopped(),
        Some(diskgraph_core::TruncationReason::ByteLimit),
        "must observe real byte exhaustion, not scheduler expiry"
    );
    let result = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || panic!("oversized id must not reach encoded success"),
    );
    assert!(
        matches!(result, Err(ref e) if e.to_lowercase().contains("budget")),
        "oversized latest id was admitted: {result:?}"
    );
}

#[test]
fn latest_empty_result_keeps_null_and_refuses_withdrawal_after_encoding() {
    let (dir, engine, _) = native_reply_deadline_tests::published_fixture();
    let root = dir.path().join("empty");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let principal = crate::local_principal().unwrap();
    engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let text = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || {},
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], true);
    assert!(reply["data"]["snapshot_id"].is_null());
    let result = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || engine.control_store().unwrap().revoke_policy().unwrap(),
    );
    assert!(
        matches!(result, Err(ref e) if e.contains("permission_denied")),
        "empty latest bypassed withdrawal: {result:?}"
    );
}

#[test]
fn latest_cannot_accept_metadata_grant_withdrawn_and_restored_after_encoding() {
    let (dir, engine, _) = native_reply_deadline_tests::published_fixture();
    let root = dir.path().join("root").canonicalize().unwrap();
    let principal = crate::local_principal().unwrap();
    let scope = engine
        .list_scopes(&principal, &engine.policy_authorizer().unwrap())
        .unwrap()
        .pop()
        .unwrap()
        .scope_id;
    let native_watch = engine
        .control_store()
        .unwrap()
        .watch_authorization_withdrawal(
            &principal,
            &scope,
            &diskgraph_core::Permission::MetadataRead,
        )
        .unwrap()
        .is_some();
    let result = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || {
            let mut control = engine.control_store().unwrap();
            control
                .revoke_grant(
                    &principal,
                    &diskgraph_core::Permission::MetadataRead,
                    &scope,
                )
                .unwrap();
            let policy_version = control.policy_version().unwrap();
            control
                .upsert_grant(&diskgraph_core::Grant {
                    principal,
                    scope,
                    permission: diskgraph_core::Permission::MetadataRead,
                    policy_version,
                })
                .unwrap();
        },
    );
    // 未知原生通知能力沿用 Engine 的代次冲突拒绝，不能宣称已观察到精确撤权。
    let expected = if native_watch {
        "permission_denied"
    } else {
        "conflict"
    };
    assert!(
        matches!(result, Err(ref e) if e.contains(expected)),
        "latest accepted withdrawn and restored metadata: {result:?}"
    );
}

#[test]
fn latest_ignores_unrelated_invalid_grants_and_scope_display_documents() {
    let (dir, engine, snapshot) = native_reply_deadline_tests::published_fixture();
    let database =
        rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    database.execute("INSERT INTO grants (principal_id,permission,scope_id,policy_version) VALUES ('unrelated',X'FF','unrelated',1)", []).unwrap();
    database
        .execute(
            "UPDATE scopes SET root_display=?1",
            ["x".repeat(2 * 1024 * 1024)],
        )
        .unwrap();
    assert!(engine.policy_authorizer().is_err());
    let root = dir.path().join("root").canonicalize().unwrap();
    let result = native_latest::query(
        &engine,
        &root,
        Instant::now() + Duration::from_secs(1),
        || {},
    )
    .unwrap();
    let reply: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(reply["data"]["snapshot_id"], snapshot);
}
