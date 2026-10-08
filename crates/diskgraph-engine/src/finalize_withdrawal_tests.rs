//! 最终响应授权回调期间的真实负向见证，不把恢复后的 grant 当作从未撤权。
use crate::{EngineError, relation_request_tests::published_authorization_fixture};
use diskgraph_core::{
    Authorizer, BusinessError, Decision, Grant, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

/// 用独立真实控制连接撤销并恢复同一 grant，保留原能力快照的允许结果。
struct WithdrawAndRestore {
    policy: PolicyAuthorizer,
    control: RefCell<diskgraph_store::ControlStore>,
    calls: Cell<u32>,
}

impl Authorizer for WithdrawAndRestore {
    fn decide(
        &self,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Decision {
        self.calls.set(self.calls.get() + 1);
        let mut control = self.control.borrow_mut();
        let version = control.policy_version().unwrap();
        control.revoke_grant(principal, permission, scope).unwrap();
        control
            .upsert_grant(&Grant {
                principal: principal.clone(),
                permission: *permission,
                scope: scope.clone(),
                policy_version: version,
            })
            .unwrap();
        self.policy.decide(principal, permission, scope)
    }
}

#[test]
fn final_response_refuses_grant_withdrawn_and_restored_in_its_capability_callback() {
    let (dir, engine, principal, scope, revision) = published_authorization_fixture();
    let authorizer = WithdrawAndRestore {
        policy: engine.policy_authorizer().unwrap(),
        control: RefCell::new(
            diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap(),
        ),
        calls: Cell::new(0),
    };
    let result = engine.finalize_revision_read_until(
        revision,
        &principal,
        &authorizer,
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(
        authorizer.calls.get(),
        1,
        "must reach the actual withdrawal callback: {result:?}"
    );
    assert_eq!(
        engine
            .control()
            .unwrap()
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(true),
        "grant must actually be restored"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(
                BusinessError::PermissionDenied | BusinessError::Conflict
            ))
        ),
        "withdrawal followed by restoration escaped final authorization: {result:?}"
    );
}
