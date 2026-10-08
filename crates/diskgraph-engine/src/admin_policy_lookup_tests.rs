//! 管理员授权的实时精确匹配与损坏数据拒绝回归。
use crate::relation_request_tests::published_authorization_fixture;
use crate::{EngineError, admin_scope};
use diskgraph_core::{BusinessError, Permission};

#[test]
fn display_initial_callback_server_replacement_refuses_consumer() {
    struct ReplaceServer {
        path: std::path::PathBuf,
        calls: std::cell::Cell<u32>,
    }
    impl diskgraph_core::Authorizer for ReplaceServer {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.calls.get() == 0 {
                rusqlite::Connection::open(&self.path)
                    .unwrap()
                    .execute("UPDATE server SET server_id='srv-replaced' WHERE id=1", [])
                    .unwrap();
            }
            self.calls.set(self.calls.get() + 1);
            diskgraph_core::Decision::Allowed
        }
    }
    let (dir, engine, principal, _, revision) = published_authorization_fixture();
    let authorizer = ReplaceServer {
        path: dir.path().join("data/diskgraph-control.sqlite"),
        calls: std::cell::Cell::new(0),
    };
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_display_reader_bounded(
        revision,
        &principal,
        &authorizer,
        diskgraph_core::QueryBudget {
            deadline_ms: 1000,
            ..diskgraph_core::QueryBudget::default()
        },
        |_, _, _| {
            entered.set(true);
            Ok(crate::RevisionDisplayCompletion::Truncated)
        },
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "replacement server: {result:?}"
    );
    assert!(
        !entered.get(),
        "replacement identity entered display consumer"
    );
}

#[test]
fn display_initial_callback_control_contention_remains_nonblocking() {
    struct HoldControl<'a> {
        engine: &'a crate::Engine,
        held: std::cell::RefCell<Option<std::sync::MutexGuard<'a, diskgraph_store::ControlStore>>>,
    }
    impl diskgraph_core::Authorizer for HoldControl<'_> {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            *self.held.borrow_mut() = Some(
                self.engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
                    .expect("initial callback must not hold control"),
            );
            diskgraph_core::Decision::Allowed
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    let authorizer = HoldControl {
        engine: &engine,
        held: std::cell::RefCell::new(None),
    };
    let started = std::time::Instant::now();
    let result = engine.with_authorized_revision_display_reader_bounded(
        revision,
        &principal,
        &authorizer,
        diskgraph_core::QueryBudget {
            deadline_ms: 1000,
            ..diskgraph_core::QueryBudget::default()
        },
        |_, _, _| panic!("contended initial authorization entered consumer"),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "contended callback: {result:?}"
    );
    assert!(
        authorizer.held.borrow().is_some(),
        "callback must retain actual competing guard"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(500),
        "postcallback admission waited for original query deadline"
    );
}

#[test]
fn display_initial_callback_can_reenter_control_and_revoke() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        reentered: std::cell::Cell<bool>,
        calls: std::cell::Cell<u32>,
        mutation: u8,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.calls.get() == 0
                && let Ok(mut control) = self
                    .engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
            {
                self.reentered.set(true);
                match self.mutation {
                    1 => control.revoke_grant(principal, permission, scope).unwrap(),
                    2 => control.revoke_scope(scope).unwrap(),
                    _ => {}
                }
            }
            self.calls.set(self.calls.get() + 1);
            self.policy.decide(principal, permission, scope)
        }
    }
    for completion in [
        crate::RevisionDisplayCompletion::Complete,
        crate::RevisionDisplayCompletion::Truncated,
    ] {
        for mutation in 0..=2 {
            let (_dir, engine, principal, _, revision) = published_authorization_fixture();
            let authorizer = Callback {
                engine: &engine,
                policy: engine.policy_authorizer().unwrap(),
                reentered: std::cell::Cell::new(false),
                calls: std::cell::Cell::new(0),
                mutation,
            };
            let entered = std::cell::Cell::new(false);
            let result = engine.with_authorized_revision_display_reader_bounded(
                revision,
                &principal,
                &authorizer,
                diskgraph_core::QueryBudget {
                    deadline_ms: 1000,
                    ..diskgraph_core::QueryBudget::default()
                },
                |_, _, _| {
                    entered.set(true);
                    Ok(completion)
                },
            );
            assert!(
                authorizer.reentered.get(),
                "display initial callback held control"
            );
            if mutation == 0 {
                result.unwrap();
                assert!(entered.get());
            } else {
                assert!(
                    matches!(
                        result,
                        Err(EngineError::Business(BusinessError::PermissionDenied))
                    ),
                    "stale initial capability: {result:?}"
                );
                assert!(!entered.get());
            }
        }
    }
}

#[test]
fn display_initial_late_capability_cannot_enter_consumer() {
    struct Late {
        denied: bool,
    }
    impl diskgraph_core::Authorizer for Late {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            std::thread::sleep(std::time::Duration::from_millis(150));
            if self.denied {
                diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::NoMatchingGrant)
            } else {
                diskgraph_core::Decision::Allowed
            }
        }
    }
    for denied in [false, true] {
        let (_dir, engine, principal, _, revision) = published_authorization_fixture();
        let result = engine.with_authorized_revision_display_reader_bounded(
            revision,
            &principal,
            &Late { denied },
            diskgraph_core::QueryBudget {
                deadline_ms: 100,
                ..diskgraph_core::QueryBudget::default()
            },
            |_, _, _| panic!("late initial capability entered consumer"),
        );
        let expected = if denied {
            BusinessError::PermissionDenied
        } else {
            BusinessError::BudgetExceeded
        };
        assert!(
            matches!(result, Err(EngineError::Business(ref error)) if *error == expected),
            "late initial capability: {result:?}"
        );
    }
}

#[test]
fn display_terminal_callback_can_reenter_control_and_revoke() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        reentered: std::cell::Cell<bool>,
        calls: std::cell::Cell<u32>,
        revoke: bool,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.calls.get() == 1
                && let Ok(mut control) = self
                    .engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
            {
                self.reentered.set(true);
                if self.revoke {
                    control.revoke_grant(principal, permission, scope).unwrap();
                }
            }
            self.calls.set(self.calls.get() + 1);
            self.policy.decide(principal, permission, scope)
        }
    }
    for completion in [
        crate::RevisionDisplayCompletion::Complete,
        crate::RevisionDisplayCompletion::Truncated,
    ] {
        for revoke in [false, true] {
            let (_dir, engine, principal, _, revision) = published_authorization_fixture();
            let authorizer = Callback {
                engine: &engine,
                policy: engine.policy_authorizer().unwrap(),
                reentered: std::cell::Cell::new(false),
                calls: std::cell::Cell::new(0),
                revoke,
            };
            let entered = std::cell::Cell::new(false);
            let result = engine.with_authorized_revision_display_reader_bounded(
                revision,
                &principal,
                &authorizer,
                diskgraph_core::QueryBudget {
                    deadline_ms: 1000,
                    ..diskgraph_core::QueryBudget::default()
                },
                |_, _, _| {
                    entered.set(true);
                    Ok(completion)
                },
            );
            assert!(entered.get());
            assert!(
                authorizer.reentered.get(),
                "display terminal callback held control"
            );
            if revoke {
                assert!(
                    matches!(
                        result,
                        Err(EngineError::Business(BusinessError::PermissionDenied))
                    ),
                    "stale allowed capability: {result:?}"
                );
            } else {
                result.unwrap();
            }
        }
    }
}

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
        "expired-unused-revision",
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
    let reader_result =
        engine.require_reader_capability_until(&Expired, &principal, &scope, Some(0), deadline);
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
        let (_dir, engine, principal, scope, revision) = published_authorization_fixture();
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
            engine.require_reader_capability_until(
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
                    revision,
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
    let policy = engine.policy_authorizer().unwrap();
    // 在真实秒的 500–550ms 区间开始，避免旧夹具在 999ms 开始而先于 consumer 到期。
    // 每次 sleep 后重读墙钟；不重试请求、不更改 token 的固定到期时间或一秒查询预算。
    let preparation_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let now = loop {
        assert!(
            std::time::Instant::now() < preparation_deadline,
            "expiry fixture could not enter the real clock phase"
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let phase = u64::from(now.subsec_millis());
        if (500..=550).contains(&phase) {
            break now;
        }
        let wait = if phase < 500 {
            500 - phase
        } else {
            1500 - phase
        };
        std::thread::sleep(std::time::Duration::from_millis(wait));
    };
    let authorizer = Fixed {
        policy,
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
    let policy = engine.policy_authorizer().unwrap();
    // 与 revision 夹具一样先选择真实秒中段，避免初始授权先于消费跨过到期边界。
    // sleep 后重读墙钟；原 token 到期时间与查询预算固定，不重试请求。
    let preparation_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let now = loop {
        assert!(
            std::time::Instant::now() < preparation_deadline,
            "expiry fixture could not enter the real clock phase"
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let phase = u64::from(now.subsec_millis());
        if (500..=550).contains(&phase) {
            break now;
        }
        let wait = if phase < 500 {
            500 - phase
        } else {
            1500 - phase
        };
        std::thread::sleep(std::time::Duration::from_millis(wait));
    };
    let authorizer = Fixed {
        policy,
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
    assert!(
        entered.get(),
        "expiry fixture never entered consumer: {result:?}"
    );
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
    // 只推进本测试线程的授权墙钟；固定token到期值与原单调查询预算均不刷新。
    let clock = crate::authority_expiry_clock::AuthorityExpiryClock::install(99);
    let authorizer = Fixed {
        policy: engine.policy_authorizer().unwrap(),
        expiry: 100,
    };
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_display_reader_bounded(
        revision,
        &principal,
        &authorizer,
        diskgraph_core::QueryBudget::default(),
        |_, _, reads| {
            assert_eq!(
                crate::authority_expiry_clock::AuthorityExpiryClock::now(),
                Some(99)
            );
            entered.set(true);
            clock.advance(authorizer.expiry);
            assert!(
                std::time::Instant::now() < reads.deadline(),
                "original query budget must remain live"
            );
            Ok(crate::RevisionDisplayCompletion::Complete)
        },
    );
    assert!(entered.get(), "consumer was not entered: {result:?}");
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired terminal authorization: {result:?}"
    );
}

#[test]
fn revision_owner_callback_can_reenter_control_and_observes_revocation() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        read_succeeded: std::cell::Cell<bool>,
        revoke: bool,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if let Ok(mut control) = self
                .engine
                .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
            {
                self.read_succeeded.set(true);
                if self.revoke {
                    control.revoke_grant(principal, permission, scope).unwrap();
                }
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for revoke in [false, true] {
        let (_dir, engine, principal, scope, revision) = published_authorization_fixture();
        let server = engine
            .control_store()
            .unwrap()
            .existing_server_id()
            .unwrap();
        let authorizer = Callback {
            engine: &engine,
            policy: engine.policy_authorizer().unwrap(),
            read_succeeded: std::cell::Cell::new(false),
            revoke,
        };
        let result = engine.authorize_revision_owner_until(
            revision,
            Some((server.as_str().to_owned(), scope.as_str().to_owned())),
            Some(&scope),
            &principal,
            &authorizer,
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        );
        assert!(
            authorizer.read_succeeded.get(),
            "owner callback held control mutex"
        );
        if revoke {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "revoked capability: {result:?}"
            );
        } else {
            assert_eq!(result.unwrap(), scope);
        }
    }
}

#[test]
fn revision_reader_initial_callback_can_reenter_control() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        read_succeeded: std::cell::Cell<bool>,
        calls: std::cell::Cell<u32>,
        revoke: bool,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.calls.get() == 0
                && let Ok(mut control) = self
                    .engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
            {
                self.read_succeeded.set(true);
                if self.revoke {
                    control.revoke_grant(principal, permission, scope).unwrap();
                }
            }
            self.calls.set(self.calls.get() + 1);
            self.policy.decide(principal, permission, scope)
        }
    }
    for revoke in [false, true] {
        let (_dir, engine, principal, _, revision) = published_authorization_fixture();
        let authorizer = Callback {
            engine: &engine,
            policy: engine.policy_authorizer().unwrap(),
            read_succeeded: std::cell::Cell::new(false),
            calls: std::cell::Cell::new(0),
            revoke,
        };
        let entered = std::cell::Cell::new(false);
        let result = engine.with_authorized_revision_reader(
            revision,
            &principal,
            &authorizer,
            1000,
            |_, _, _| {
                entered.set(true);
                Ok(())
            },
        );
        assert!(
            authorizer.read_succeeded.get(),
            "initial capability callback ran under control lock"
        );
        if revoke {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "revoked capability: {result:?}"
            );
            assert!(!entered.get());
        } else {
            result.unwrap();
            assert!(entered.get());
        }
    }
}

#[test]
fn revision_reader_preserves_explicit_denial_after_callback_deadline() {
    struct LateDenied;
    impl diskgraph_core::Authorizer for LateDenied {
        fn decide(
            &self,
            _: &diskgraph_core::PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            std::thread::sleep(std::time::Duration::from_millis(150));
            diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::NoMatchingGrant)
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    let result: Result<(), EngineError> = engine.with_authorized_revision_reader(
        revision,
        &principal,
        &LateDenied,
        100,
        |_, _, _| panic!("denied request entered consumer"),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "observed denial changed: {result:?}"
    );
}

#[test]
fn revision_reader_terminal_callback_can_reenter_control_and_revoke() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        read_succeeded: std::cell::Cell<bool>,
        calls: std::cell::Cell<u32>,
        revoke: bool,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.calls.get() == 1
                && let Ok(mut control) = self
                    .engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
            {
                self.read_succeeded.set(true);
                if self.revoke {
                    control.revoke_grant(principal, permission, scope).unwrap();
                }
            }
            self.calls.set(self.calls.get() + 1);
            self.policy.decide(principal, permission, scope)
        }
    }
    for revoke in [false, true] {
        let (_dir, engine, principal, _, revision) = published_authorization_fixture();
        let authorizer = Callback {
            engine: &engine,
            policy: engine.policy_authorizer().unwrap(),
            read_succeeded: std::cell::Cell::new(false),
            calls: std::cell::Cell::new(0),
            revoke,
        };
        let entered = std::cell::Cell::new(false);
        let result = engine.with_authorized_revision_reader(
            revision,
            &principal,
            &authorizer,
            1000,
            |_, _, _| {
                entered.set(true);
                Ok(())
            },
        );
        assert!(
            authorizer.read_succeeded.get(),
            "terminal capability callback ran under control lock"
        );
        if revoke {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "revoked capability: {result:?}"
            );
            assert!(entered.get());
        } else {
            result.unwrap();
            assert!(entered.get());
        }
    }
}
