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

#[test]
fn final_response_quarantine_denial_precedes_unrelated_policy_generation_conflict() {
    struct QuarantineDuringDecision {
        policy: PolicyAuthorizer,
        control: RefCell<diskgraph_store::ControlStore>,
        graph: RefCell<diskgraph_store::SqliteSnapshotStore>,
        server: String,
        calls: Cell<u32>,
    }
    impl Authorizer for QuarantineDuringDecision {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            self.calls.set(self.calls.get() + 1);
            let mut control = self.control.borrow_mut();
            let version = control.policy_version().unwrap();
            control
                .upsert_grant(&Grant {
                    principal: PrincipalId::new("unrelated-quarantine-subject").unwrap(),
                    permission: Permission::MetadataRead,
                    scope: scope.clone(),
                    policy_version: version,
                })
                .unwrap();
            assert_eq!(
                self.graph
                    .borrow_mut()
                    .isolate_unconfirmed_revision_roots(&self.server, scope.as_str(), None)
                    .unwrap(),
                1
            );
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, scope, revision) = published_authorization_fixture();
    let authorizer = QuarantineDuringDecision {
        policy: engine.policy_authorizer().unwrap(),
        control: RefCell::new(
            diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap(),
        ),
        graph: RefCell::new(
            diskgraph_store::SqliteSnapshotStore::open(&dir.path().join("data/diskgraph.sqlite"))
                .unwrap(),
        ),
        server: engine.server_id().unwrap().as_str().to_owned(),
        calls: Cell::new(0),
    };
    let result = engine.finalize_revision_read_until(
        revision,
        &principal,
        &authorizer,
        Instant::now() + Duration::from_secs(1),
    );
    assert_eq!(authorizer.calls.get(), 1);
    assert_eq!(
        engine
            .control()
            .unwrap()
            .live_permission(&principal, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(true),
        "original subject remains authorized"
    );
    assert!(
        authorizer
            .graph
            .borrow()
            .revision_ownership(revision)
            .unwrap()
            .is_none()
    );
    assert!(
        authorizer
            .graph
            .borrow()
            .revision_ownership_for_audit(revision)
            .unwrap()
            .is_some()
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "confirmed revision quarantine must precede unknown generation conflict: {result:?}"
    );
}
