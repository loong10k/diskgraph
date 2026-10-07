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

#[test]
fn ordinary_requirement_refuses_token_expired_while_waiting_for_control() {
    struct Expiring {
        policy: diskgraph_core::PolicyAuthorizer,
        expiry: u64,
        entered_live: std::cell::Cell<bool>,
    }
    impl diskgraph_core::Authorizer for Expiring {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.entered_live.set(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    < self.expiry,
            );
            self.policy.decide(principal, permission, scope)
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    let (_dir, engine, principal, scope, _) = published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 2;
    let engine = std::sync::Arc::new(engine);
    let holder = engine.clone();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        let guard = holder.control_store().unwrap();
        ready_tx.send(()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        std::thread::sleep(
            std::time::Duration::from_secs(expiry).saturating_sub(now)
                + std::time::Duration::from_millis(30),
        );
        drop(guard);
    });
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let authorizer = Expiring {
        policy,
        expiry,
        entered_live: std::cell::Cell::new(false),
    };
    let result = engine.require(&authorizer, &principal, &Permission::MetadataRead, &scope);
    owner.join().unwrap();
    assert!(
        authorizer.entered_live.get(),
        "fixture must obtain capability before expiry"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired capability returned: {result:?}"
    );
}

#[test]
fn initial_revision_authorization_rejects_expired_allowed_capability() {
    struct Expired;
    impl diskgraph_core::Authorizer for Expired {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            diskgraph_core::Decision::Allowed
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(0)
        }
    }
    let (_dir, engine, principal, scope, _) = published_authorization_fixture();
    let server = engine
        .control_store()
        .unwrap()
        .existing_server_id()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let owner_result = engine.authorize_revision_owner_until(
        Some((server.as_str().to_owned(), scope.as_str().to_owned())),
        Some(&scope),
        &principal,
        &Expired,
        deadline,
    );
    assert!(
        matches!(
            owner_result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired owner authorization: {owner_result:?}"
    );
    let control = engine.control_store().unwrap();
    let reader_result = crate::Engine::require_reader_capability_until(
        &control,
        &Expired,
        &principal,
        &scope,
        Some(0),
        deadline,
    );
    assert!(
        matches!(
            reader_result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired reader authorization: {reader_result:?}"
    );
}

#[test]
fn initial_revision_authorization_refuses_allowed_callback_returning_after_expiry() {
    struct Late {
        expiry: u64,
        entered_live: std::cell::Cell<bool>,
    }
    impl diskgraph_core::Authorizer for Late {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            self.entered_live.set(now.as_secs() < self.expiry);
            std::thread::sleep(
                std::time::Duration::from_secs(self.expiry).saturating_sub(now)
                    + std::time::Duration::from_millis(30),
            );
            diskgraph_core::Decision::Allowed
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    for reader in [false, true] {
        let (_dir, engine, principal, scope, _) = published_authorization_fixture();
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2;
        let authorizer = Late {
            expiry,
            entered_live: std::cell::Cell::new(false),
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let result = if reader {
            let control = engine.control_store().unwrap();
            crate::Engine::require_reader_capability_until(
                &control,
                &authorizer,
                &principal,
                &scope,
                Some(expiry),
                deadline,
            )
        } else {
            let server = engine
                .control_store()
                .unwrap()
                .existing_server_id()
                .unwrap();
            engine
                .authorize_revision_owner_until(
                    Some((server.as_str().to_owned(), scope.as_str().to_owned())),
                    Some(&scope),
                    &principal,
                    &authorizer,
                    deadline,
                )
                .map(|_| ())
        };
        assert!(
            authorizer.entered_live.get(),
            "callback must start before fixed expiry"
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "reader={reader}: {result:?}"
        );
    }
}

#[test]
fn revision_reader_expiry_getter_can_reenter_control() {
    struct Getter<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        read_succeeded: std::cell::Cell<bool>,
    }
    impl diskgraph_core::Authorizer for Getter<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.policy.decide(principal, permission, scope)
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            self.read_succeeded.set(
                self.engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
                    .is_ok(),
            );
            None
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    let authorizer = Getter {
        engine: &engine,
        policy: engine.policy_authorizer().unwrap(),
        read_succeeded: std::cell::Cell::new(false),
    };
    engine
        .with_authorized_revision_reader(revision, &principal, &authorizer, 1000, |_, _, _| Ok(()))
        .unwrap();
    assert!(
        authorizer.read_succeeded.get(),
        "expiry getter ran under control lock"
    );
}

#[test]
fn revision_reader_refuses_token_expired_during_consumer() {
    struct Fixed {
        policy: diskgraph_core::PolicyAuthorizer,
        expiry: u64,
    }
    impl diskgraph_core::Authorizer for Fixed {
        fn decide(
            &self,
            p: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.policy.decide(p, permission, scope)
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    // 留足原始一秒执行预算：在墙钟秒的后半段开始，消费只等待下一个固定秒边界。
    let mut now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    if now.subsec_millis() < 500 {
        std::thread::sleep(std::time::Duration::from_millis(
            500 - u64::from(now.subsec_millis()),
        ));
        now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
    }
    let authorizer = Fixed {
        policy: engine.policy_authorizer().unwrap(),
        expiry: now.as_secs() + 1,
    };
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_reader(
        revision,
        &principal,
        &authorizer,
        1000,
        |_, _, deadline| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            assert!(
                now.as_secs() < authorizer.expiry,
                "consumer must start before expiry"
            );
            entered.set(true);
            std::thread::sleep(
                std::time::Duration::from_secs(authorizer.expiry).saturating_sub(now)
                    + std::time::Duration::from_millis(20),
            );
            assert!(
                std::time::Instant::now() < deadline,
                "fixture must retain original query budget"
            );
            Ok(())
        },
    );
    assert!(entered.get());
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired terminal authorization: {result:?}"
    );
}

#[test]
fn display_reader_rejects_expired_token_before_preparing_canvas() {
    struct Expired;
    impl diskgraph_core::Authorizer for Expired {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            diskgraph_core::Decision::Allowed
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(0)
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    for completion in [
        crate::RevisionDisplayCompletion::Complete,
        crate::RevisionDisplayCompletion::Truncated,
    ] {
        let entered = std::cell::Cell::new(false);
        let result = engine.with_authorized_revision_display_reader_bounded(
            revision,
            &principal,
            &Expired,
            diskgraph_core::QueryBudget::default(),
            |_, _, _| {
                entered.set(true);
                Ok(completion)
            },
        );
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "expired display: {result:?}"
        );
        assert!(!entered.get(), "expired request prepared canvas");
    }
}

#[test]
fn display_reader_refuses_token_expired_during_consumer() {
    struct Fixed {
        policy: diskgraph_core::PolicyAuthorizer,
        expiry: u64,
    }
    impl diskgraph_core::Authorizer for Fixed {
        fn decide(
            &self,
            p: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.policy.decide(p, permission, scope)
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    // 留足原始一秒执行预算：在墙钟秒的后半段开始，消费只等待下一个固定秒边界。
    let mut now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    if now.subsec_millis() < 500 {
        std::thread::sleep(std::time::Duration::from_millis(
            500 - u64::from(now.subsec_millis()),
        ));
        now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
    }
    let authorizer = Fixed {
        policy: engine.policy_authorizer().unwrap(),
        expiry: now.as_secs() + 1,
    };
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_display_reader_bounded(
        revision,
        &principal,
        &authorizer,
        diskgraph_core::QueryBudget::default(),
        |_, _, reads| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            assert!(
                now.as_secs() < authorizer.expiry,
                "consumer must start before expiry"
            );
            entered.set(true);
            std::thread::sleep(
                std::time::Duration::from_secs(authorizer.expiry).saturating_sub(now)
                    + std::time::Duration::from_millis(20),
            );
            assert!(
                std::time::Instant::now() < reads.deadline(),
                "original query budget must remain live"
            );
            Ok(crate::RevisionDisplayCompletion::Truncated)
        },
    );
    assert!(entered.get());
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired terminal authorization: {result:?}"
    );
}

#[test]
fn display_complete_reader_refuses_token_expired_during_consumer() {
    struct Fixed {
        policy: diskgraph_core::PolicyAuthorizer,
        expiry: u64,
    }
    impl diskgraph_core::Authorizer for Fixed {
        fn decide(
            &self,
            p: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.policy.decide(p, permission, scope)
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    // 留足原始一秒执行预算：在墙钟秒的后半段开始，消费只等待下一个固定秒边界。
    let mut now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    if now.subsec_millis() < 500 {
        std::thread::sleep(std::time::Duration::from_millis(
            500 - u64::from(now.subsec_millis()),
        ));
        now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
    }
    let authorizer = Fixed {
        policy: engine.policy_authorizer().unwrap(),
        expiry: now.as_secs() + 1,
    };
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_display_reader_bounded(
        revision,
        &principal,
        &authorizer,
        diskgraph_core::QueryBudget::default(),
        |_, _, reads| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            assert!(
                now.as_secs() < authorizer.expiry,
                "consumer must start before expiry"
            );
            entered.set(true);
            std::thread::sleep(
                std::time::Duration::from_secs(authorizer.expiry).saturating_sub(now)
                    + std::time::Duration::from_millis(20),
            );
            assert!(
                std::time::Instant::now() < reads.deadline(),
                "original query budget must remain live"
            );
            Ok(crate::RevisionDisplayCompletion::Complete)
        },
    );
    assert!(entered.get());
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired terminal authorization: {result:?}"
    );
}
