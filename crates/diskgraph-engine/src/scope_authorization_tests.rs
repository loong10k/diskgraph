//! scope 列表的锁外能力回调与最终实时授权交集回归。
use crate::relation_request_tests::published_authorization_fixture;
use crate::{Engine, EngineError, admin_scope};
use diskgraph_core::{
    Authorizer, Decision, DenyReason, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};
use diskgraph_store::ControlStore;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

#[test]
fn list_scopes_does_not_reuse_admin_fallback_after_callback_revocation() {
    struct Revokes {
        policy: PolicyAuthorizer,
        control: RefCell<ControlStore>,
    }
    impl Authorizer for Revokes {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            if scope != &admin_scope() {
                let mut control = self.control.borrow_mut();
                control
                    .revoke_grant(principal, permission, &admin_scope())
                    .unwrap();
                control.revoke_grant(principal, permission, scope).unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, _, _) = published_authorization_fixture();
    let authorizer = Revokes {
        policy: engine.policy_authorizer().unwrap(),
        control: RefCell::new(
            ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite")).unwrap(),
        ),
    };
    assert!(
        engine
            .list_scopes(&principal, &authorizer)
            .unwrap()
            .is_empty(),
        "revoked admin fallback exposed scopes"
    );
}

#[test]
fn list_scopes_rechecks_earlier_scope_after_later_capability_callback() {
    struct RevokesEarlier {
        policy: PolicyAuthorizer,
        control: RefCell<ControlStore>,
        first: ScopeId,
        last: ScopeId,
    }
    impl Authorizer for RevokesEarlier {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            if scope == &admin_scope() {
                return Decision::Denied(DenyReason::Disabled);
            }
            if scope == &self.last {
                self.control
                    .borrow_mut()
                    .revoke_grant(principal, permission, &self.first)
                    .unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, _, _) = published_authorization_fixture();
    let root = dir.path().join("second-root");
    std::fs::create_dir(&root).unwrap();
    engine
        .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let scopes = engine.control_store().unwrap().list_scopes().unwrap();
    assert_eq!(scopes.len(), 2);
    let authorizer = RevokesEarlier {
        policy: engine.policy_authorizer().unwrap(),
        control: RefCell::new(
            ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite")).unwrap(),
        ),
        first: scopes[0].scope_id.clone(),
        last: scopes[1].scope_id.clone(),
    };
    let allowed = engine.list_scopes(&principal, &authorizer).unwrap();
    assert_eq!(allowed.len(), 1);
    assert_eq!(allowed[0].scope_id, authorizer.last);
}

#[test]
fn list_scope_capability_callbacks_can_read_engine_control() {
    struct Reads<'a> {
        engine: &'a Engine,
        policy: PolicyAuthorizer,
        all_reads_succeeded: Cell<bool>,
    }
    impl Authorizer for Reads<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            let read = self
                .engine
                .control_until(Instant::now() + Duration::from_millis(50))
                .and_then(|control| control.existing_server_id().map_err(EngineError::from));
            self.all_reads_succeeded
                .set(self.all_reads_succeeded.get() && read.is_ok());
            self.policy.decide(principal, permission, scope)
        }
    }
    let (_dir, engine, principal, _, _) = published_authorization_fixture();
    let authorizer = Reads {
        engine: &engine,
        policy: engine.policy_authorizer().unwrap(),
        all_reads_succeeded: Cell::new(true),
    };
    assert_eq!(
        engine.list_scopes(&principal, &authorizer).unwrap().len(),
        1
    );
    assert!(
        authorizer.all_reads_succeeded.get(),
        "scope callbacks held the shared control lock"
    );
}

#[test]
fn list_scopes_refuses_capability_result_returned_after_token_expiry() {
    struct Expires {
        policy: PolicyAuthorizer,
        expiry: u64,
    }
    impl Authorizer for Expires {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            let decision = self.policy.decide(principal, permission, scope);
            if scope != &admin_scope() {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap();
                let remaining = Duration::from_secs(self.expiry).saturating_sub(now);
                std::thread::sleep(remaining + Duration::from_millis(20));
            }
            decision
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    let (_dir, engine, principal, _, _) = published_authorization_fixture();
    let authorizer = Expires {
        policy: engine.policy_authorizer().unwrap(),
        expiry: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 1,
    };
    assert!(matches!(
        engine.list_scopes(&principal, &authorizer),
        Err(EngineError::Business(
            diskgraph_core::BusinessError::PermissionDenied
        ))
    ));
}

#[test]
fn list_scopes_preserves_admin_only_fallback_and_trusted_no_policy_mode() {
    for legacy in [false, true] {
        let (dir, engine, principal, scope, _) = published_authorization_fixture();
        let policy;
        if legacy {
            policy = engine.policy_authorizer().unwrap();
            rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap()
                .execute_batch("DELETE FROM grants; DELETE FROM policy;")
                .unwrap();
        } else {
            engine
                .control_store()
                .unwrap()
                .revoke_grant(&principal, &Permission::MetadataRead, &scope)
                .unwrap();
            policy = engine.policy_authorizer().unwrap();
        }
        let allowed = engine.list_scopes(&principal, &policy).unwrap();
        assert_eq!(allowed.len(), 1);
        assert_eq!(allowed[0].scope_id, scope);
    }
}

#[test]
fn scope_registration_capability_callbacks_do_not_hold_control_lock() {
    struct Reads<'a> {
        engine: &'a Engine,
        policy: PolicyAuthorizer,
        all_reads_succeeded: Cell<bool>,
        calls: Cell<usize>,
    }
    impl Authorizer for Reads<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            self.calls.set(self.calls.get() + 1);
            let read = self
                .engine
                .control_until(Instant::now() + Duration::from_millis(50))
                .and_then(|control| control.existing_server_id().map_err(EngineError::from));
            self.all_reads_succeeded
                .set(self.all_reads_succeeded.get() && read.is_ok());
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, _, _) = published_authorization_fixture();
    let root = dir.path().join("registered-root");
    std::fs::create_dir(&root).unwrap();
    let authorizer = Reads {
        engine: &engine,
        policy: engine.policy_authorizer().unwrap(),
        all_reads_succeeded: Cell::new(true),
        calls: Cell::new(0),
    };
    let scope = engine
        .register_scope(&root, &principal, &authorizer)
        .unwrap();
    assert_eq!(authorizer.calls.get(), 2);
    assert!(
        authorizer.all_reads_succeeded.get(),
        "registration callback held control lock"
    );
    assert_eq!(engine.scope(&scope).unwrap().scope_id, scope);
}

#[test]
fn scope_registration_refuses_admin_revocation_during_second_callback() {
    struct Revokes {
        policy: PolicyAuthorizer,
        control: RefCell<ControlStore>,
        calls: Cell<usize>,
    }
    impl Authorizer for Revokes {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == 2 {
                self.control
                    .borrow_mut()
                    .revoke_grant(principal, permission, scope)
                    .unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, _, _) = published_authorization_fixture();
    let root = dir.path().join("refused-root");
    std::fs::create_dir(&root).unwrap();
    let authorizer = Revokes {
        policy: engine.policy_authorizer().unwrap(),
        control: RefCell::new(
            ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite")).unwrap(),
        ),
        calls: Cell::new(0),
    };
    assert!(matches!(
        engine.register_scope(&root, &principal, &authorizer),
        Err(EngineError::Business(
            diskgraph_core::BusinessError::PermissionDenied
        ))
    ));
    assert_eq!(authorizer.calls.get(), 2);
    assert_eq!(
        engine.control_store().unwrap().list_scopes().unwrap().len(),
        1
    );
}
