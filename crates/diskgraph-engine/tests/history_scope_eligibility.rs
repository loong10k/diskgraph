//! D34 原生平台旧历史离线元数据夹具；不创建非法目录，Linux 另测真实原始目录。
#![cfg(any(unix, windows))]
#[path = "query_request_budget/callback_authorizer.rs"]
mod callback_authorizer;
#[path = "history_scope_eligibility/fixture.rs"]
mod fixture;
#[cfg(target_os = "linux")]
#[path = "history_scope_eligibility/linux_fs_tests.rs"]
mod linux_fs_tests;
use callback_authorizer::CallbackAuthorizer;
use diskgraph_core::{BusinessError, Permission, QueryBudget};
use diskgraph_engine::EngineError;
use diskgraph_store::{ControlStore, SqliteSnapshotStore, StoreError};
use fixture::{ScopeHistory, legacy_graph};
use std::path::Path;
use std::time::{Duration, Instant};

fn assert_scope_incompatible(value: &serde_json::Value) {
    assert_eq!(value["incompatible"], "different_root", "{value}");
    assert_eq!(value["scope_changed"], true, "{value}");
}

#[test]
fn authorized_distinct_actual_scopes_are_not_growth_compatible() {
    let f = ScopeHistory::new();
    assert!(
        f.growth("before", "after").unwrap().is_none(),
        "both sides authorized, but their actual scopes differ"
    );
    assert!(
        f.engine
            .growth_between("before", "after", Path::new("item"))
            .unwrap()
            .is_none(),
        "trusted API must also preserve actual scope compatibility"
    );
}

#[test]
fn authorized_distinct_actual_scopes_have_changes_incompatibility() {
    let f = ScopeHistory::new();
    let value = f
        .engine
        .revision_changes_until(
            "before",
            "after",
            QueryBudget::default(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    assert_scope_incompatible(&value);
    assert_scope_incompatible(&f.engine.revision_changes("before", "after").unwrap());
}

#[test]
fn same_actual_scope_history_keeps_positive_delta_and_size_changes() {
    let f = ScopeHistory::new();
    f.publish("same-scope", 0, 200, 3, None);
    assert_eq!(
        f.growth("before", "same-scope")
            .unwrap()
            .unwrap()
            .delta_bytes,
        100
    );
    assert_eq!(
        f.engine
            .growth_between("before", "same-scope", Path::new("item"))
            .unwrap()
            .unwrap()
            .delta_bytes,
        100
    );
    let changes = f.engine.revision_changes("before", "same-scope").unwrap();
    assert!(changes["incompatible"].is_null());
    assert_eq!(changes["size_changed"], 1);
}

#[test]
fn generic_authorized_comparison_remains_available_across_scopes_and_roots() {
    let f = ScopeHistory::new();
    let policy = f.engine.policy_authorizer().unwrap();
    let report = f
        .engine
        .compare_revisions_until(
            "before",
            "after",
            0,
            QueryBudget::default(),
            &f.principal,
            &policy,
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.summary.different, 1);
    let root = f.directory.path().join("distinct-display-root");
    std::fs::create_dir(&root).unwrap();
    let scope = f
        .engine
        .register_scope(&root, &f.principal, &policy)
        .unwrap();
    let graph = legacy_graph("other-root", &root, 300, 4);
    let mut store =
        SqliteSnapshotStore::open(&f.directory.path().join("data/diskgraph.sqlite")).unwrap();
    store
        .append_staging_nodes("other-root", &graph.nodes)
        .unwrap();
    store
        .publish_revision_owned(
            "other-root",
            &graph,
            "other-root",
            4,
            Some((f.engine.server_id().unwrap().as_str(), scope.as_str())),
        )
        .unwrap();
    let report = f
        .engine
        .compare_revisions_until(
            "before",
            "other-root",
            0,
            QueryBudget::default(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    assert_ne!(report.left_root, report.right_root);
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.summary.different, 1);
}

#[test]
fn missing_grant_on_either_side_is_denied_instead_of_incompatible() {
    for side in 0..2 {
        let f = ScopeHistory::new();
        f.engine
            .control_store()
            .unwrap()
            .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scopes[side])
            .unwrap();
        assert!(matches!(
            f.growth("before", "after"),
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
        let result = f.engine.revision_changes_until(
            "before",
            "after",
            QueryBudget::default(),
            &f.principal,
            &f.engine.policy_authorizer().unwrap(),
            Instant::now() + Duration::from_secs(30),
        );
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
    }
}

#[test]
fn foreign_server_owned_history_is_denied_before_comparability() {
    let f = ScopeHistory::new();
    f.publish("foreign", 1, 200, 3, Some("foreign-server"));
    assert!(matches!(
        f.growth("before", "foreign"),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    let result = f.engine.revision_changes_until(
        "before",
        "foreign",
        QueryBudget::default(),
        &f.principal,
        &f.engine.policy_authorizer().unwrap(),
        Instant::now() + Duration::from_secs(30),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn scope_incompatible_growth_still_rechecks_terminal_revocation() {
    for (side, terminal_call) in [(0, 3), (1, 3), (0, 5), (1, 5)] {
        let f = ScopeHistory::new();
        let control = f.directory.path().join("data/diskgraph-control.sqlite");
        let scope = f.scopes[side].clone();
        let policy = CallbackAuthorizer::new(f.engine.policy_authorizer().unwrap(), move |call| {
            if call == terminal_call {
                ControlStore::open(&control)
                    .unwrap()
                    .revoke_scope(&scope)
                    .unwrap();
            }
        });
        let result = f.engine.growth_between_until(
            "before",
            "after",
            Path::new("item"),
            QueryBudget::default(),
            &f.principal,
            &policy,
            Instant::now() + Duration::from_secs(30),
        );
        assert!(policy.calls.get() >= terminal_call);
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
    }
}

#[test]
fn scope_incompatible_growth_rejects_late_capability() {
    let f = ScopeHistory::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    let policy = CallbackAuthorizer::new(f.engine.policy_authorizer().unwrap(), move |call| {
        if call == 3 {
            // 原数据期限仍有效；仅能力回调超出独立 50 ms 窗口。
            std::thread::sleep(Duration::from_millis(80));
        }
    });
    let result = f.engine.growth_between_until(
        "before",
        "after",
        Path::new("item"),
        QueryBudget::default(),
        &f.principal,
        &policy,
        deadline,
    );
    assert!(policy.calls.get() >= 3);
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn scope_incompatible_growth_still_cannot_fit_null_in_one_byte_budget() {
    let f = ScopeHistory::new();
    let result = f.engine.growth_between_until(
        "before",
        "after",
        Path::new("item"),
        QueryBudget {
            max_response_bytes: 1,
            ..QueryBudget::default()
        },
        &f.principal,
        &f.engine.policy_authorizer().unwrap(),
        Instant::now() + Duration::from_secs(30),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
            | Err(EngineError::Store(StoreError::BudgetExceeded))
    ));
}

#[test]
fn scope_incompatible_changes_still_rechecks_terminal_revocation() {
    for (side, terminal_call) in [(0, 3), (1, 3), (0, 5), (1, 5)] {
        let f = ScopeHistory::new();
        let control = f.directory.path().join("data/diskgraph-control.sqlite");
        let scope = f.scopes[side].clone();
        let policy = CallbackAuthorizer::new(f.engine.policy_authorizer().unwrap(), move |call| {
            if call == terminal_call {
                ControlStore::open(&control)
                    .unwrap()
                    .revoke_scope(&scope)
                    .unwrap();
            }
        });
        let result = f.engine.revision_changes_until(
            "before",
            "after",
            QueryBudget::default(),
            &f.principal,
            &policy,
            Instant::now() + Duration::from_secs(30),
        );
        assert!(policy.calls.get() >= terminal_call);
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
    }
}

#[test]
fn scope_incompatible_changes_cannot_hide_response_budget_or_late_capability() {
    let f = ScopeHistory::new();
    let result = f.engine.revision_changes_until(
        "before",
        "after",
        QueryBudget {
            max_response_bytes: 1,
            ..QueryBudget::default()
        },
        &f.principal,
        &f.engine.policy_authorizer().unwrap(),
        Instant::now() + Duration::from_secs(30),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
            | Err(EngineError::Store(StoreError::BudgetExceeded))
    ));
    let deadline = Instant::now() + Duration::from_secs(30);
    let policy = CallbackAuthorizer::new(f.engine.policy_authorizer().unwrap(), move |call| {
        if call == 3 {
            // 原数据期限仍有效；仅能力回调超出独立 50 ms 窗口。
            std::thread::sleep(Duration::from_millis(80));
        }
    });
    let result = f.engine.revision_changes_until(
        "before",
        "after",
        QueryBudget::default(),
        &f.principal,
        &policy,
        deadline,
    );
    assert!(policy.calls.get() >= 3);
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}
