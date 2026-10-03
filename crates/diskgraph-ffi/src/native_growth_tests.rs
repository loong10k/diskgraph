//! 旧 UniFFI growth 真实数据库预算回归；来源：D25 / Q-02。

use crate::{growth_json, scan_native_json};
use diskgraph_core::{QueryBudget, ResourceLocator};
use serde_json::Value;

fn fixture() -> (tempfile::TempDir, tempfile::TempDir, String, String, String) {
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let database = data
        .path()
        .join("graph.sqlite")
        .to_str()
        .unwrap()
        .to_owned();
    let scan: Value = serde_json::from_str(&scan_native_json(
        database.clone(),
        root.path().to_str().unwrap().to_owned(),
    ))
    .unwrap();
    assert_eq!(scan["ok"], true, "{scan}");
    let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
    let locator = serde_json::to_string(&ResourceLocator::NativePath(
        root.path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned(),
    ))
    .unwrap();
    (root, data, database, snapshot, locator)
}

#[test]
fn legacy_growth_refuses_oversized_rows_before_returning_the_envelope() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute(
        "UPDATE nodes SET name=?1 WHERE snapshot_id=?2 AND parent_id IS NULL",
        rusqlite::params!["x".repeat(100_000), snapshot],
    )
    .unwrap();
    let text = growth_json(database, snapshot.clone(), snapshot, locator);
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], false, "oversized growth must be rejected");
    assert!(reply["error"].as_str().unwrap().contains("budget"));
    assert!(reply.get("data").is_none());
    assert!(text.len() <= QueryBudget::default().max_response_bytes);
}

#[test]
fn legacy_growth_cumulative_two_sided_rows_cannot_reset_raw_budget() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute(
        "UPDATE nodes SET name=?1 WHERE snapshot_id=?2 AND parent_id IS NULL",
        rusqlite::params!["x".repeat(40_000), snapshot],
    )
    .unwrap();
    let reply: Value =
        serde_json::from_str(&growth_json(database, snapshot.clone(), snapshot, locator)).unwrap();
    assert_eq!(
        reply["ok"], false,
        "both decoded rows share a single budget"
    );
    assert!(reply["error"].as_str().unwrap().contains("budget"));
}

#[test]
fn legacy_growth_admits_raw_columns_before_decoding_corrupt_kind() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute(
        "UPDATE nodes SET name=?1,kind='broken' WHERE snapshot_id=?2",
        rusqlite::params!["x".repeat(100_000), snapshot],
    )
    .unwrap();
    let reply: Value =
        serde_json::from_str(&growth_json(database, snapshot.clone(), snapshot, locator)).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"].as_str().unwrap().contains("budget"),
        "{reply}"
    );
}

#[test]
fn legacy_growth_refuses_oversized_snapshot_before_json_decode() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute(
        "UPDATE snapshots SET snapshot_json=?1 WHERE id=?2",
        rusqlite::params!["x".repeat(100_000), snapshot],
    )
    .unwrap();
    let reply: Value =
        serde_json::from_str(&growth_json(database, snapshot.clone(), snapshot, locator)).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(
        reply["error"].as_str().unwrap().contains("budget"),
        "{reply}"
    );
}

#[test]
fn legacy_growth_limits_actual_json_escaping_and_error_diagnostics() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let db = rusqlite::Connection::open(&database).unwrap();
    for column in ["name", "kind"] {
        db.execute(
            &format!("UPDATE nodes SET {column}=?1 WHERE snapshot_id=?2"),
            rusqlite::params!["\0".repeat(11_000), snapshot],
        )
        .unwrap();
        let text = growth_json(
            database.clone(),
            snapshot.clone(),
            snapshot.clone(),
            locator.clone(),
        );
        let reply: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reply["ok"], false);
        assert!(reply.get("data").is_none());
        assert!(text.len() <= QueryBudget::default().max_response_bytes);
    }
}

#[test]
fn legacy_growth_preserves_zero_delta_and_missing_locator_null() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let reply: Value = serde_json::from_str(&growth_json(
        database.clone(),
        snapshot.clone(),
        snapshot.clone(),
        locator,
    ))
    .unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"]["delta_bytes"], "0");
    let missing =
        serde_json::to_string(&ResourceLocator::NativePath("/missing-d25-fixture".into())).unwrap();
    let reply: Value =
        serde_json::from_str(&growth_json(database, snapshot.clone(), snapshot, missing)).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["data"], Value::Null);
}

#[test]
fn legacy_growth_refuses_unbound_snapshot_and_preserves_incompatible_null() {
    let (_root, _data, database, snapshot, locator) = fixture();
    let db = rusqlite::Connection::open(&database).unwrap();
    let original: String = db
        .query_row(
            "SELECT snapshot_json FROM snapshots WHERE id=?1",
            [&snapshot],
            |row| row.get(0),
        )
        .unwrap();
    // 同快照的未知卷或未完整覆盖都必须继续保持旧 null 契约。
    for (path, value) in [("$.volume_id", "null"), ("$.coverage.complete", "false")] {
        db.execute(
            "UPDATE snapshots SET snapshot_json=json_set(?1,?2,json(?3)) WHERE id=?4",
            rusqlite::params![original, path, value, snapshot],
        )
        .unwrap();
        let reply: Value = serde_json::from_str(&growth_json(
            database.clone(),
            snapshot.clone(),
            snapshot.clone(),
            locator.clone(),
        ))
        .unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(reply["data"].is_null());
    }
    let text = growth_json(database, snapshot, "unbound-snapshot".into(), locator);
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("permission_denied")
    );
    assert!(reply.get("data").is_none());
}

#[test]
fn legacy_growth_terminal_expiry_rejects_both_data_and_null() {
    use std::time::{Duration, Instant};
    let (_root, _data, database, snapshot, locator) = fixture();
    let engine = crate::open_engine(&database).unwrap();
    let missing =
        serde_json::to_string(&ResourceLocator::NativePath("/missing-d25-fixture".into())).unwrap();
    for locator in [&locator, &missing] {
        let deadline = Instant::now() + Duration::from_secs(2);
        let called = std::cell::Cell::new(false);
        let error =
            crate::native_growth::query(&engine, [&snapshot, &snapshot], locator, deadline, || {
                called.set(true);
                std::thread::sleep(
                    deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
                );
            })
            .unwrap_err();
        assert!(called.get(), "must reach the encoded reply boundary");
        assert!(
            error.contains("budget") || error.contains("timeout"),
            "{error}"
        );
    }
}

#[test]
fn legacy_growth_terminal_authorization_checks_both_scopes_and_grants() {
    use diskgraph_core::Permission;
    for side in 0..2 {
        for scope_revocation in [false, true] {
            let (_root, _data, database, first, locator) = fixture();
            let other = tempfile::tempdir().unwrap();
            let scan: Value = serde_json::from_str(&scan_native_json(
                database.clone(),
                other.path().to_str().unwrap().into(),
            ))
            .unwrap();
            assert_eq!(scan["ok"], true, "{scan}");
            let second = scan["data"]["snapshot_id"].as_str().unwrap();
            let snapshots = [first.as_str(), second];
            let engine = crate::open_engine(&database).unwrap();
            let principal = crate::local_principal().unwrap();
            let revision = engine
                .revision_reader()
                .unwrap()
                .revision_for_snapshot(snapshots[side])
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
            let deadline = diskgraph_core::query_deadline(QueryBudget::default()).unwrap();
            let called = std::cell::Cell::new(false);
            let error = crate::native_growth::query(&engine, snapshots, &locator, deadline, || {
                called.set(true);
                let mut control = engine.control_store().unwrap();
                if scope_revocation {
                    control.revoke_scope(&scope).unwrap();
                } else {
                    control
                        .revoke_grant(&principal, &Permission::MetadataRead, &scope)
                        .unwrap();
                }
            })
            .unwrap_err();
            assert!(called.get());
            assert!(
                error.contains("permission_denied"),
                "side={side},scope={scope_revocation}: {error}"
            );
        }
    }
}
