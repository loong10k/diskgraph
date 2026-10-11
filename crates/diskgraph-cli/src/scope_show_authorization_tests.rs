//! 实际scope show不能绕过主体权限、撤权和dispatch原期限；仅隔离元数据，无扫描替身。
use crate::cli::Cli;
use crate::dispatch::dispatch;
use clap::Parser;
use diskgraph_core::{Authorizer, BusinessError, Decision, Permission, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use diskgraph_store::ControlStore;
use std::cell::Cell;
use std::time::{Duration, Instant};

fn fixture() -> (tempfile::TempDir, Engine, PrincipalId, ScopeId, Cli) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("private_root");
    std::fs::create_dir(&root).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("local-user").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let cli = Cli::parse_from([
        "diskgraph",
        "--json",
        "scope",
        "show",
        "--scope",
        scope.as_str(),
    ]);
    (directory, engine, principal, scope, cli)
}

#[test]
fn scope_show_refuses_ungranted_principal_without_root_display() {
    let (_directory, engine, _owner, _scope, cli) = fixture();
    let actor = PrincipalId::new("ungranted-reader").unwrap();
    let mut output = Vec::new();
    let result = dispatch(
        &engine,
        &cli,
        &actor,
        &engine.policy_authorizer().unwrap(),
        &mut output,
        Instant::now() + Duration::from_secs(5),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "ungranted scope metadata escaped: {result:?}, {output:?}"
    );
    assert!(output.is_empty());
}

#[test]
fn scope_show_keeps_allowed_fields_but_refuses_expired_original_request() {
    let (_directory, engine, actor, scope, cli) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let mut output = Vec::new();
    dispatch(
        &engine,
        &cli,
        &actor,
        &policy,
        &mut output,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let document: serde_json::Value = serde_json::from_str(&output[0]).unwrap();
    assert_eq!(document["data"]["scope_id"], scope.as_str());
    assert_eq!(document["data"]["revoked"], false);
    output.clear();
    let result = dispatch(&engine, &cli, &actor, &policy, &mut output, Instant::now());
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(output.is_empty());
}

/// 第二次能力回调在独立连接撤销scope；旧能力快照不能使已编码根路径逃逸。
struct RevokingAuthorizer {
    policy: diskgraph_core::PolicyAuthorizer,
    control: std::cell::RefCell<ControlStore>,
    calls: Cell<usize>,
}
impl Authorizer for RevokingAuthorizer {
    fn decide(&self, actor: &PrincipalId, permission: &Permission, scope: &ScopeId) -> Decision {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 2 {
            self.control.borrow_mut().revoke_scope(scope).unwrap();
        }
        self.policy.decide(actor, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
}

#[test]
fn scope_show_rechecks_live_revocation_after_encoding_before_output() {
    let (directory, engine, actor, _scope, cli) = fixture();
    let authorizer = RevokingAuthorizer {
        policy: engine.policy_authorizer().unwrap(),
        control: std::cell::RefCell::new(
            ControlStore::open(&directory.path().join("data/diskgraph-control.sqlite")).unwrap(),
        ),
        calls: Cell::new(0),
    };
    let mut output = Vec::new();
    let result = dispatch(
        &engine,
        &cli,
        &actor,
        &authorizer,
        &mut output,
        Instant::now() + Duration::from_secs(5),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "revoked scope metadata escaped: {result:?}, {output:?}"
    );
    assert_eq!(authorizer.calls.get(), 2);
    assert!(output.is_empty());
}

/// 能力回调超过批准的50ms窗口；即使返回Allowed也不能交付已编码结果。
struct LateAuthorizer {
    policy: diskgraph_core::PolicyAuthorizer,
    calls: Cell<usize>,
}
impl Authorizer for LateAuthorizer {
    fn decide(&self, actor: &PrincipalId, permission: &Permission, scope: &ScopeId) -> Decision {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() == 2 {
            std::thread::sleep(Duration::from_millis(60));
        }
        self.policy.decide(actor, permission, scope)
    }
    fn policy_version(&self) -> u64 {
        self.policy.policy_version()
    }
}

#[test]
fn scope_show_refuses_late_allowed_capability_after_encoding() {
    let (_directory, engine, actor, _scope, cli) = fixture();
    let authorizer = LateAuthorizer {
        policy: engine.policy_authorizer().unwrap(),
        calls: Cell::new(0),
    };
    let mut output = Vec::new();
    let result = dispatch(
        &engine,
        &cli,
        &actor,
        &authorizer,
        &mut output,
        Instant::now() + Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert_eq!(authorizer.calls.get(), 2);
    assert!(output.is_empty());
}
