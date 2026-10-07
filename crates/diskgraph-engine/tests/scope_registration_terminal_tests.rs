//! 注册写入前的请求能力终检；来源：OpenSpec SC-01 / SC-04。
use diskgraph_core::{
    Authorizer, BusinessError, Decision, DenyReason, Permission, PrincipalId, ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};
use std::cell::Cell;

/// 模拟初检后失效的请求能力；来源：原生 Rust 授权回归。
struct ExpiringCapability(Cell<usize>);

impl Authorizer for ExpiringCapability {
    fn decide(&self, _: &PrincipalId, _: &Permission, _: &ScopeId) -> Decision {
        let count = self.0.get();
        self.0.set(count + 1);
        if count == 0 {
            Decision::Allowed
        } else {
            Decision::Denied(DenyReason::Disabled)
        }
    }
}

#[test]
fn registration_rechecks_capability_before_persisting_scope() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("registrar").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let result = engine.register_scope(&root, &principal, &ExpiringCapability(Cell::new(0)));
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "{result:?}"
    );
    assert!(
        engine
            .list_scopes(&principal, &engine.policy_authorizer().unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn registration_enforces_the_original_expiry_even_if_a_capability_decision_allows() {
    /// 分离静态能力与绝对认证期限，验证事务不把 Allowed 当作无限委派。
    struct ExpiredAuthentication;
    impl Authorizer for ExpiredAuthentication {
        fn decide(&self, _: &PrincipalId, _: &Permission, _: &ScopeId) -> Decision {
            Decision::Allowed
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(0)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("expired-registrar").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let result = engine.register_scope(&root, &principal, &ExpiredAuthentication);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "{result:?}"
    );
    assert!(
        engine
            .control_store()
            .unwrap()
            .list_scopes()
            .unwrap()
            .is_empty()
    );
}
