//! D34 真实 Linux 非 UTF-8 原始目录；macOS 不执行，不将平台跳过计为 Linux 验收。
use super::fixture::ScopeHistory;
use diskgraph_core::QueryBudget;
use std::time::{Duration, Instant};

#[test]
fn linux_real_raw_roots_with_distinct_owned_scopes_have_no_growth() {
    let f = ScopeHistory::new_live_linux();
    assert!(f.growth("before", "after").unwrap().is_none());
}

#[test]
fn linux_real_raw_roots_with_distinct_owned_scopes_have_changes_diagnostic() {
    let f = ScopeHistory::new_live_linux();
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
    assert!(!value["incompatible"].is_null(), "{value}");
    assert_eq!(value["scope_changed"], true, "{value}");
}
