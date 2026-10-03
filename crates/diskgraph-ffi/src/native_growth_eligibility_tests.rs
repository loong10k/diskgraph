//! D33 旧绑定通过合法旧/import 节点验证未知大小与实际类型替换。
use crate::{growth_json, scan_native_json};
use diskgraph_core::{QueryBudget, ResourceLocator};
use diskgraph_store::SqliteSnapshotStore;
use serde_json::Value;
use std::time::{Duration, Instant};

fn imported_pair(
    left_bad: bool,
    read_error: bool,
    target: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    String,
    String,
    String,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("item"), b"payload").unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let scanned: Value = serde_json::from_str(&scan_native_json(
        database.clone(),
        root.path().to_str().unwrap().into(),
    ))
    .unwrap();
    assert_eq!(scanned["ok"], true, "{scanned}");
    let revision = scanned["data"]["revision"].as_str().unwrap();
    let engine = crate::open_engine(&database).unwrap();
    let original = engine.load_revision(revision).unwrap();
    let locator = serde_json::to_string(
        &original
            .nodes
            .iter()
            .find(|node| {
                if target.is_empty() {
                    node.parent_id.is_none()
                } else {
                    node.name == target
                }
            })
            .unwrap()
            .locator,
    )
    .unwrap();
    let mut left = original.clone();
    let mut right = original;
    let mut store = SqliteSnapshotStore::open(std::path::Path::new(&database)).unwrap();
    let owner = store.revision_ownership(revision).unwrap().unwrap();
    for (graph, id, offset, bad) in [
        (&mut left, "before", 1, left_bad),
        (&mut right, "after", 2, !left_bad),
    ] {
        graph.snapshot.id = id.into();
        graph.snapshot.captured_at_unix_ms += offset;
        if bad {
            let node = graph
                .nodes
                .iter_mut()
                .find(|node| {
                    if target.is_empty() {
                        node.parent_id.is_none()
                    } else {
                        node.name == target
                    }
                })
                .unwrap();
            node.subtree_bytes = 99999;
            if read_error {
                node.read_error = true
            } else {
                node.size_known = false
            }
        }
        store.append_staging_nodes(id, &graph.nodes).unwrap();
        store
            .publish_revision_owned(
                id,
                graph,
                id,
                graph.snapshot.captured_at_unix_ms,
                Some((&owner.0, &owner.1)),
            )
            .unwrap();
    }
    (
        root,
        data,
        database,
        left.snapshot.id,
        right.snapshot.id,
        locator,
    )
}

#[test]
fn ffi_growth_unknown_and_read_error_on_either_side_returns_success_null() {
    for left_bad in [false, true] {
        for read_error in [false, true] {
            let (_root, _data, database, left, right, locator) =
                imported_pair(left_bad, read_error, "item");
            let reply: Value =
                serde_json::from_str(&growth_json(database, left, right, locator)).unwrap();
            assert_eq!(reply["ok"], true, "{reply}");
            assert!(
                reply["data"].is_null(),
                "left_bad={left_bad},read_error={read_error}: {reply}"
            );
        }
    }
}

#[test]
fn ffi_growth_real_same_path_file_directory_replacement_refuses_delta() {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let item = root.path().join("item");
    std::fs::write(&item, b"payload").unwrap();
    let before: Value = serde_json::from_str(&scan_native_json(
        database.clone(),
        root.path().to_str().unwrap().into(),
    ))
    .unwrap();
    assert_eq!(before["ok"], true, "{before}");
    std::fs::remove_file(&item).unwrap();
    std::fs::create_dir(&item).unwrap();
    std::fs::write(item.join("child"), b"replacement child").unwrap();
    let after: Value = serde_json::from_str(&scan_native_json(
        database.clone(),
        root.path().to_str().unwrap().into(),
    ))
    .unwrap();
    assert_eq!(after["ok"], true, "{after}");
    let locator = serde_json::to_string(&ResourceLocator::NativePath(
        item.canonicalize().unwrap().to_str().unwrap().into(),
    ))
    .unwrap();
    let reply: Value = serde_json::from_str(&growth_json(
        database,
        before["data"]["snapshot_id"].as_str().unwrap().into(),
        after["data"]["snapshot_id"].as_str().unwrap().into(),
        locator,
    ))
    .unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply["data"].is_null(), "{reply}");
}

#[test]
fn ffi_unknown_growth_still_rechecks_terminal_grant_and_deadline() {
    for expire in [false, true] {
        let (_root, _data, database, left, right, locator) = imported_pair(true, false, "item");
        let engine = crate::open_engine(&database).unwrap();
        let principal = crate::local_principal().unwrap();
        let scope = engine
            .authorize_revision(
                None,
                &left,
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let called = std::cell::Cell::new(false);
        let result =
            crate::native_growth::query(&engine, [&left, &right], &locator, deadline, || {
                called.set(true);
                if expire {
                    std::thread::sleep(
                        deadline.saturating_duration_since(Instant::now())
                            + Duration::from_millis(1),
                    );
                } else {
                    engine
                        .control_store()
                        .unwrap()
                        .revoke_grant(
                            &principal,
                            &diskgraph_core::Permission::MetadataRead,
                            &scope,
                        )
                        .unwrap();
                }
            });
        assert!(called.get());
        let error = result.unwrap_err();
        assert!(
            if expire {
                error.contains("timeout") || error.contains("budget")
            } else {
                error.contains("permission_denied")
            },
            "{error}"
        );
    }
}

#[test]
fn ffi_unknown_growth_keeps_borrowed_row_budget_before_null_result() {
    let (_root, _data, database, left, right, locator) = imported_pair(true, false, "item");
    let db = rusqlite::Connection::open(&database).unwrap();
    // 合法未知节点的 JSON 中加入巨大名称，仍须先按字段成本准入。
    let old: String = db
        .query_row(
            "SELECT node_json FROM nodes WHERE snapshot_id=?1 AND name='item'",
            [&left],
            |row| row.get(0),
        )
        .unwrap();
    let mut node: diskgraph_core::DiskNode = serde_json::from_str(&old).unwrap();
    node.name = "x".repeat(QueryBudget::default().max_response_bytes + 1);
    db.execute(
        "UPDATE nodes SET node_json=?1 WHERE snapshot_id=?2 AND name='item'",
        rusqlite::params![serde_json::to_string(&node).unwrap(), left],
    )
    .unwrap();
    let reply: Value = serde_json::from_str(&growth_json(database, left, right, locator)).unwrap();
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(reply["error"].as_str().unwrap().contains("budget"));
}

#[test]
fn ffi_root_growth_unknown_or_read_error_is_success_null() {
    for read_error in [false, true] {
        let (_root, _data, database, left, right, locator) = imported_pair(true, read_error, "");
        let reply: Value =
            serde_json::from_str(&growth_json(database, left, right, locator)).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(reply["data"].is_null(), "{reply}");
    }
}
