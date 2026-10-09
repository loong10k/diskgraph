//! 未知原生通知平台的本地提交负向见证；真实SQLite到期不等于已知撤权事实消失。
use crate::request_metadata_withdrawals::RequestMetadataWithdrawals;
use crate::{Engine, EngineConfig, EngineError};
use diskgraph_core::{BusinessError, Grant, Locator, Permission, PrincipalId, ScopeId};
use diskgraph_store::ControlStore;
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, Engine, PrincipalId, ScopeId) {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-commit-reader").unwrap();
    let scope = {
        let mut control = engine.control_store().unwrap();
        // 真实内存数据库没有可订阅的原生文件身份，确定进入未知通知分支。
        *control = ControlStore::open_in_memory().unwrap();
        control.publish_policy_version(1).unwrap();
        let locator = Locator::from_native_path(directory.path());
        let scope = control.register_scope(&locator, None).unwrap();
        control
            .upsert_grant(&Grant {
                principal: principal.clone(),
                permission: Permission::MetadataRead,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
        assert!(
            control
                .watch_authorization_withdrawal(&principal, &scope, &Permission::MetadataRead)
                .unwrap()
                .is_none()
        );
        scope
    };
    (directory, engine, principal, scope)
}

fn capture(engine: &Engine, actor: &PrincipalId, scope: &ScopeId) -> RequestMetadataWithdrawals {
    RequestMetadataWithdrawals::capture(&engine.control_store().unwrap(), actor, &[scope]).unwrap()
}

fn expired_sql(engine: &Engine) -> Result<(), EngineError> {
    engine
        .control_store()
        .unwrap()
        .with_read_deadline(Instant::now() - Duration::from_millis(1), |_| {
            panic!("expired SQL consumer must never run")
        })
        .map_err(crate::relation_request::terminal_control_error)
}

#[test]
fn committed_local_withdrawal_remains_conflict_after_terminal_sql_expiry() {
    let (_directory, engine, actor, scope) = fixture();
    let observation = capture(&engine, &actor, &scope);
    {
        let mut control = engine.control_store().unwrap();
        control
            .revoke_grant(&actor, &Permission::MetadataRead, &scope)
            .unwrap();
        control
            .upsert_grant(&Grant {
                principal: actor,
                permission: Permission::MetadataRead,
                scope,
                policy_version: 1,
            })
            .unwrap();
    }
    assert!(matches!(
        observation.prioritize_terminal_budget(&engine, expired_sql(&engine)),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
}

#[test]
fn absent_authorization_change_preserves_original_budget_failure() {
    let (_directory, engine, actor, scope) = fixture();
    let observation = capture(&engine, &actor, &scope);
    assert!(matches!(
        observation.prioritize_terminal_budget(&engine, expired_sql(&engine)),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn unrelated_server_write_does_not_create_authorization_conflict() {
    let (_directory, engine, actor, scope) = fixture();
    let observation = capture(&engine, &actor, &scope);
    engine.control_store().unwrap().ensure_server().unwrap();
    assert!(matches!(
        observation.prioritize_terminal_budget(&engine, expired_sql(&engine)),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn committed_scope_withdrawal_survives_expired_terminal_sql() {
    let (_directory, engine, actor, scope) = fixture();
    let observation = capture(&engine, &actor, &scope);
    engine
        .control_store()
        .unwrap()
        .revoke_scope(&scope)
        .unwrap();
    assert!(matches!(
        observation.prioritize_terminal_budget(&engine, expired_sql(&engine)),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
}

#[test]
fn replacing_control_connection_does_not_borrow_its_committed_facts() {
    let (_directory, engine, actor, scope) = fixture();
    let observation = capture(&engine, &actor, &scope);
    {
        let mut control = engine.control_store().unwrap();
        let mut replacement = ControlStore::open_in_memory().unwrap();
        let replacement_scope = replacement
            .register_scope(&Locator::from_native_path(&std::env::temp_dir()), None)
            .unwrap();
        replacement.revoke_scope(&replacement_scope).unwrap();
        *control = replacement;
    }
    assert!(matches!(
        observation.prioritize_terminal_budget(&engine, expired_sql(&engine)),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}
