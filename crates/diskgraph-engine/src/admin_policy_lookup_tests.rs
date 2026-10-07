//! 管理员授权的实时精确匹配与损坏数据拒绝回归。
use crate::relation_request_tests::published_authorization_fixture;
use crate::{EngineError, admin_scope};
use diskgraph_core::{BusinessError, Permission};

#[test]
fn admin_lookup_observes_independent_revocation_and_corrupt_text() {
    for corrupt in [false, true] {
        let (dir, engine, principal, _, _) = published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        engine
            .require(&policy, &principal, &Permission::ScopeAdmin, &admin_scope())
            .unwrap();
        let independent =
            rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
        if corrupt {
            independent
                .execute_batch(
                    "INSERT INTO grants VALUES(CAST(X'FF' AS TEXT),'scope:admin','unrelated',1);",
                )
                .unwrap();
        } else {
            independent
                .execute(
                    "DELETE FROM grants WHERE principal_id=?1 AND permission=?2 AND scope_id=?3",
                    rusqlite::params![
                        principal.as_str(),
                        Permission::ScopeAdmin.wire_name(),
                        admin_scope().as_str()
                    ],
                )
                .unwrap();
        }
        let result = engine.require(&policy, &principal, &Permission::ScopeAdmin, &admin_scope());
        if corrupt {
            assert!(matches!(result, Err(EngineError::Store(_))));
        } else {
            assert!(matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ));
        }
    }
}

#[test]
fn ordinary_requirement_callback_can_read_engine_control_without_reentrant_lock() {
    struct ReadsEngine<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        read_succeeded: std::cell::Cell<bool>,
    }
    impl diskgraph_core::Authorizer for ReadsEngine<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
            let read = self
                .engine
                .control_until(deadline)
                .and_then(|control| control.existing_server_id().map_err(EngineError::from));
            self.read_succeeded.set(read.is_ok());
            self.policy.decide(principal, permission, scope)
        }
    }
    let (_dir, engine, principal, scope, _) = published_authorization_fixture();
    let authorizer = ReadsEngine {
        engine: &engine,
        policy: engine.policy_authorizer().unwrap(),
        read_succeeded: std::cell::Cell::new(false),
    };
    engine
        .require(&authorizer, &principal, &Permission::MetadataRead, &scope)
        .unwrap();
    assert!(
        authorizer.read_succeeded.get(),
        "host capability callback executed under the engine control lock"
    );
}

#[test]
fn ordinary_requirement_observes_revocation_committed_inside_capability_callback() {
    struct Revokes {
        policy: diskgraph_core::PolicyAuthorizer,
        control: std::cell::RefCell<diskgraph_store::ControlStore>,
    }
    impl diskgraph_core::Authorizer for Revokes {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.control
                .borrow_mut()
                .revoke_grant(principal, permission, scope)
                .unwrap();
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, scope, _) = published_authorization_fixture();
    let authorizer = Revokes {
        policy: engine.policy_authorizer().unwrap(),
        control: std::cell::RefCell::new(
            diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap(),
        ),
    };
    assert!(matches!(
        engine.require(&authorizer, &principal, &Permission::MetadataRead, &scope),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}
