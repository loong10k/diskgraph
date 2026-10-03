//! Q-04 / D34：公开旧 FFI growth 的实际 scope 资格与授权边界。
//! Unix 与 Windows 使用本平台原始编码构造离线旧元数据，不声称创建真实原目录。
use crate::native_growth_scope_fixture::NativeGrowthScopeFixture;
use serde_json::Value;
use std::time::{Duration, Instant};

fn growth(fixture: &NativeGrowthScopeFixture, after: &str, root: bool) -> Value {
    serde_json::from_str(&crate::growth_json(
        fixture.database.clone(),
        "before".into(),
        after.into(),
        fixture.locator_json(root),
    ))
    .unwrap()
}

#[test]
fn ffi_authorized_different_actual_scopes_return_null_for_root_and_child() {
    let fixture = NativeGrowthScopeFixture::new();
    for root in [false, true] {
        let reply = growth(&fixture, "after", root);
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(reply["data"].is_null(), "actual scopes differ: {reply}");
    }
}

#[test]
fn ffi_same_actual_scope_preserves_positive_growth() {
    let fixture = NativeGrowthScopeFixture::new();
    fixture.publish("same-scope", 0, 200, 3, None);
    for root in [false, true] {
        let reply = growth(&fixture, "same-scope", root);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["data"]["before"]["subtree_bytes"], 100, "{reply}");
        assert_eq!(reply["data"]["after"]["subtree_bytes"], 200, "{reply}");
        assert_eq!(reply["data"]["delta_bytes"], "100", "{reply}");
    }
}

#[test]
fn ffi_different_scope_history_requires_both_grants() {
    for side in [0, 1] {
        let fixture = NativeGrowthScopeFixture::new();
        fixture.revoke(side);
        let reply = growth(&fixture, "after", false);
        assert_eq!(reply["ok"], false, "side={side}: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("permission_denied"),
            "{reply}"
        );
    }
}

#[test]
fn ffi_foreign_server_history_is_denied_before_comparability() {
    let fixture = NativeGrowthScopeFixture::new();
    fixture.publish("foreign", 1, 200, 3, Some("foreign-server"));
    let reply = growth(&fixture, "foreign", false);
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("permission_denied"),
        "{reply}"
    );
}

#[test]
fn ffi_incompatible_scope_null_still_rechecks_each_terminal_grant() {
    for side in [0, 1] {
        let fixture = NativeGrowthScopeFixture::new();
        let called = std::cell::Cell::new(false);
        let result = crate::native_growth::query(
            &fixture.engine,
            ["before", "after"],
            &fixture.locator_json(false),
            Instant::now() + Duration::from_secs(30),
            || {
                called.set(true);
                fixture.revoke(side);
            },
        );
        assert!(called.get());
        let error = result.unwrap_err();
        assert!(error.contains("permission_denied"), "side={side}: {error}");
    }
}

#[test]
fn ffi_incompatible_scope_null_still_rechecks_terminal_deadline() {
    let fixture = NativeGrowthScopeFixture::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    let called = std::cell::Cell::new(false);
    let result = crate::native_growth::query(
        &fixture.engine,
        ["before", "after"],
        &fixture.locator_json(false),
        deadline,
        || {
            called.set(true);
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
        },
    );
    assert!(called.get());
    let error = result.unwrap_err();
    assert!(
        error.contains("timeout") || error.contains("budget"),
        "{error}"
    );
}
