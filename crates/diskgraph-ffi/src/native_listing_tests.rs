//! 原生目录页的真实公开入口回归；来源：D26 / Q-02。
use crate::{NativeService, children_json, scan_native_json, top_json};
use serde_json::Value;

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, String, String) {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a"), b"0123456789").unwrap();
    std::fs::write(root.path().join("b"), b"1").unwrap();
    let path = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let scan: Value = serde_json::from_str(&scan_native_json(
        path.clone(),
        root.path().to_str().unwrap().into(),
    ))
    .unwrap();
    assert_eq!(scan["ok"], true, "{scan}");
    let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
    // 分配大小可能同为一个文件系统块；固定排序键，确保被测大字段始终在页内。
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(db.execute("UPDATE nodes SET subtree_bytes=CASE name WHEN 'a' THEN 20 ELSE 10 END WHERE parent_id=1", []).unwrap(), 2);
    (root, data, path, snapshot)
}

#[test]
fn legacy_top_refuses_an_oversized_row_instead_of_returning_it() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE nodes SET name=?1 WHERE name='a'",
        ["x".repeat(100_000)],
    )
    .unwrap();
    let text = top_json(path, snapshot, 1, 1);
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], false, "oversized node must not succeed");
    assert!(reply["error"].as_str().unwrap().contains("budget"));
    assert!(text.len() <= diskgraph_core::QueryBudget::default().max_response_bytes);
}

#[test]
fn legacy_children_do_not_decode_the_continuation() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE nodes SET kind='broken' WHERE name='b'", [])
        .unwrap();
    let reply: Value =
        serde_json::from_str(&children_json(path.clone(), snapshot.clone(), 1, 0, 1)).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["items"][0]["name"], "a");
    assert_eq!(reply["data"]["next_offset"], 1);
    let included: Value = serde_json::from_str(&children_json(path, snapshot, 1, 0, 2)).unwrap();
    assert_eq!(
        included["ok"], false,
        "in-budget corrupt record must still fail"
    );
}

#[test]
fn session_children_admit_raw_fields_before_decoding_an_invalid_kind() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE nodes SET name=?1,kind='broken' WHERE name='a'",
        ["x".repeat(100_000)],
    )
    .unwrap();
    let session = NativeService::new(path).unwrap();
    let text = session.children_json(snapshot, 1, 0, 2);
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"].as_str().unwrap().contains("budget"),
        "{reply}"
    );
}

#[test]
fn invalid_legacy_limits_do_not_open_or_create_a_database() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("not-created/graph.sqlite");
    for limit in [0, 1001] {
        for top in [true, false] {
            let text = if top {
                top_json(path.to_str().unwrap().into(), "absent".into(), 1, limit)
            } else {
                children_json(path.to_str().unwrap().into(), "absent".into(), 1, 0, limit)
            };
            let reply: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(reply["ok"], false);
            assert!(
                reply["error"].as_str().unwrap().contains("limit"),
                "{reply}"
            );
            assert!(!path.parent().unwrap().exists());
        }
    }
}

#[test]
fn session_cumulative_raw_budget_keeps_prefix_and_actual_continuation() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE nodes SET name=?1 WHERE name='a'",
        ["a".repeat(40_000)],
    )
    .unwrap();
    db.execute(
        "UPDATE nodes SET name=?1,kind='broken' WHERE name='b'",
        ["b".repeat(40_000)],
    )
    .unwrap();
    let session = NativeService::new(path.clone()).unwrap();
    let reply: Value =
        serde_json::from_str(&session.children_json(snapshot.clone(), 1, 0, 2)).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(reply["data"]["next_offset"], 1);
    assert_eq!(reply["data"]["complete"], false);
    assert_eq!(reply["data"]["truncated"], "raw_byte_limit");
    let next: Value =
        serde_json::from_str(&session.children_json(snapshot.clone(), 1, 1, 1)).unwrap();
    assert_eq!(next["ok"], false, "in-budget corrupt next page must fail");
    assert!(
        next["error"]
            .as_str()
            .unwrap()
            .contains("unknown node kind")
    );
    let legacy: Value = serde_json::from_str(&children_json(path, snapshot, 1, 0, 2)).unwrap();
    assert_eq!(legacy["ok"], false);
    assert!(legacy["error"].as_str().unwrap().contains("budget"));
}

#[test]
fn listing_keeps_legacy_unknown_nodes_and_session_unknown_count() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    let id: i64 = db
        .query_row("SELECT id FROM nodes WHERE name='b'", [], |r| r.get(0))
        .unwrap();
    let engine = crate::open_engine(&path).unwrap();
    let mut node = engine
        .revision_reader()
        .unwrap()
        .node(&snapshot, id as u64)
        .unwrap()
        .unwrap();
    node.size_known = false;
    node.read_error = true;
    node.subtree_bytes = 10;
    db.execute(
        "UPDATE nodes SET kind=NULL,node_json=?1,read_error=1 WHERE id=?2",
        rusqlite::params![serde_json::to_string(&node).unwrap(), id],
    )
    .unwrap();
    db.execute(
        "UPDATE directory_counts SET unknown_count=1 WHERE parent_id=1",
        [],
    )
    .unwrap();
    let legacy: Value =
        serde_json::from_str(&top_json(path.clone(), snapshot.clone(), 1, 2)).unwrap();
    assert_eq!(legacy["ok"], true, "{legacy}");
    assert_eq!(legacy["data"].as_array().unwrap().len(), 2);
    assert_eq!(legacy["data"][1]["size_known"], false);
    let session = NativeService::new(path).unwrap();
    let reply: Value = serde_json::from_str(&session.children_json(snapshot, 1, 0, 2)).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(reply["data"]["unknown_size_count"], 1);
    assert_eq!(reply["data"]["complete"], true);
    assert!(reply["data"]["next_offset"].is_null());
}

#[test]
fn legacy_list_data_and_error_diagnostics_bound_actual_json_escaping() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    for field in ["name", "kind"] {
        db.execute(
            &format!("UPDATE nodes SET {field}=?1 WHERE subtree_bytes=20"),
            ["\0".repeat(11_000)],
        )
        .unwrap();
        for top in [true, false] {
            let text = if top {
                top_json(path.clone(), snapshot.clone(), 1, 2)
            } else {
                children_json(path.clone(), snapshot.clone(), 1, 0, 2)
            };
            let reply: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(reply["ok"], false);
            assert!(reply.get("data").is_none());
            assert!(text.len() <= diskgraph_core::QueryBudget::default().max_response_bytes);
        }
    }
}

#[test]
fn every_listing_checks_permission_cancellation_and_deadline_after_encoding() {
    use diskgraph_core::Permission;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::{Duration, Instant};
    for kind in 0..3 {
        for stop in ["scope", "grant", "cancel", "deadline"] {
            let (_root, _data, path, snapshot) = fixture();
            let engine = crate::open_engine(&path).unwrap();
            let principal = crate::local_principal().unwrap();
            let revision = engine
                .revision_reader()
                .unwrap()
                .revision_for_snapshot(&snapshot)
                .unwrap()
                .unwrap();
            let scope = engine
                .authorize_revision(
                    None,
                    &revision,
                    &principal,
                    &engine.policy_authorizer().unwrap(),
                )
                .unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let called = std::cell::Cell::new(false);
            let deadline = Instant::now() + Duration::from_secs(2);
            let result = crate::native_reply::query(
                &engine,
                &snapshot,
                deadline,
                cancel.clone(),
                |store, until| match kind {
                    0 => crate::native_listing::top(store, &snapshot, 1, 2, until),
                    1 => crate::native_listing::children(store, &snapshot, 1, 0, 2, until),
                    _ => crate::native_listing::session_children(store, &snapshot, 1, 0, 2, until),
                },
                || {
                    called.set(true);
                    match stop {
                        "scope" => engine
                            .control_store()
                            .unwrap()
                            .revoke_scope(&scope)
                            .unwrap(),
                        "grant" => engine
                            .control_store()
                            .unwrap()
                            .revoke_grant(&principal, &Permission::MetadataRead, &scope)
                            .unwrap(),
                        "cancel" => cancel.store(true, Ordering::SeqCst),
                        _ => std::thread::sleep(
                            deadline.saturating_duration_since(Instant::now())
                                + Duration::from_millis(1),
                        ),
                    }
                },
            );
            assert!(
                called.get(),
                "kind={kind},stop={stop}: must reach a successfully encoded reply"
            );
            match result {
                Ok(text) => {
                    assert_eq!(stop, "deadline");
                    assert_eq!(kind, 2, "legacy listing cannot return late success");
                    let reply: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(reply["data"]["complete"], false);
                    assert_eq!(reply["data"]["truncated"], "deadline");
                }
                Err(error) => match stop {
                    "scope" | "grant" => assert!(error.contains("permission_denied"), "{error}"),
                    "cancel" => assert!(error.contains("closed"), "{error}"),
                    _ => assert!(
                        error.contains("budget") || error.contains("timeout"),
                        "{error}"
                    ),
                },
            }
        }
    }
}

#[test]
fn session_keeps_zero_size_and_filters_negative_legacy_rows() {
    let (_root, _data, path, snapshot) = fixture();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE nodes SET subtree_bytes=CASE name WHEN 'a' THEN 0 ELSE -1 END WHERE parent_id=1",
        [],
    )
    .unwrap();
    let session = NativeService::new(path.clone()).unwrap();
    let reply: Value =
        serde_json::from_str(&session.children_json(snapshot.clone(), 1, 0, 2)).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(reply["data"]["items"][0]["name"], "a");
    assert_eq!(reply["data"]["items"][0]["subtree_bytes"], 0);
    assert_eq!(reply["data"]["complete"], true);
    assert!(reply["data"]["next_offset"].is_null());
    // 旧列表没有非负过滤；保留它原有的条目集合，避免顺带改变兼容入口。
    let legacy: Value = serde_json::from_str(&children_json(path, snapshot, 1, 0, 2)).unwrap();
    assert_eq!(legacy["ok"], true, "{legacy}");
    assert_eq!(legacy["data"]["items"].as_array().unwrap().len(), 2);
}
