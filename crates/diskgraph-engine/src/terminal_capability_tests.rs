//! 关系与历史的迟到能力、跨侧撤权与隔离优先级回归。
use crate::EngineError;
use crate::relation_request_tests::published_authorization_fixture;
use diskgraph_core::{BusinessError, Permission, PrincipalId, QueryBudget, query_deadline};
use std::cell::RefCell;

#[test]
fn initial_authorization_control_contention_expires_while_original_lock_is_held() {
    for mode in 0..5 {
        let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
        let snapshot_id = engine
            .graph()
            .unwrap()
            .revision(revision)
            .unwrap()
            .snapshot_id;
        let reader = engine.revision_reader().unwrap();
        let engine = std::sync::Arc::new(engine);
        let policy = engine.policy_authorizer().unwrap();
        let released = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let holder = engine.clone();
        let holder_released = released.clone();
        let owner = std::thread::spawn(move || {
            let guard = holder.control_store().unwrap();
            ready_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
            holder_released.store(true, std::sync::atomic::Ordering::Release);
            drop(guard);
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
        let result = match mode {
            0 => engine.authorize_snapshot_until(&snapshot_id, &principal, &policy, deadline),
            1 => engine
                .authorize_revision_until(None, revision, &principal, &policy, deadline)
                .map(|_| ()),
            4 => engine.latest_revision_until(&_scope, deadline).map(|_| ()),
            3 => engine.with_authorized_revision_reader(
                revision,
                &principal,
                &policy,
                50,
                |_, _, _| panic!("consumer must not run after initial authorization lock expiry"),
            ),
            _ => {
                let mut reads =
                    diskgraph_core::QueryReadBudget::new(QueryBudget::default(), deadline).unwrap();
                engine
                    .authorize_revision_with_budget(
                        &reader, None, revision, &principal, &policy, &mut reads,
                    )
                    .map(|_| ())
            }
        };
        let returned_while_held = !released.load(std::sync::atomic::Ordering::Acquire);
        owner.join().unwrap();
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
        assert!(
            returned_while_held,
            "request waited for the original owner past its deadline"
        );
    }
}

#[test]
fn bounded_reader_terminal_control_contention_refuses_prepared_result() {
    let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
    let engine = std::sync::Arc::new(engine);
    let policy = engine.policy_authorizer().unwrap();
    let released = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let owner = RefCell::new(None);
    let result =
        engine.with_authorized_revision_reader(revision, &principal, &policy, 100, |_, _, _| {
            let holder = engine.clone();
            let holder_released = released.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            *owner.borrow_mut() = Some(std::thread::spawn(move || {
                let guard = holder.control_store().unwrap();
                tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(300));
                holder_released.store(true, std::sync::atomic::Ordering::Release);
                drop(guard);
            }));
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            Ok(42)
        });
    let returned_while_held = !released.load(std::sync::atomic::Ordering::Acquire);
    owner
        .into_inner()
        .expect("consumer must prepare a result")
        .join()
        .unwrap();
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(
        returned_while_held,
        "terminal authorization waited past original deadline"
    );
}

#[test]
fn initial_revision_and_snapshot_authorization_do_not_return_late_allow() {
    struct ExpiringDecision {
        policy: diskgraph_core::PolicyAuthorizer,
        deadline: std::time::Instant,
        entered_live: std::cell::Cell<bool>,
        denied: bool,
    }
    impl diskgraph_core::Authorizer for ExpiringDecision {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.entered_live
                .set(std::time::Instant::now() < self.deadline);
            std::thread::sleep(
                self.deadline
                    .saturating_duration_since(std::time::Instant::now())
                    + std::time::Duration::from_millis(1),
            );
            if self.denied {
                diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::Disabled)
            } else {
                self.policy.decide(principal, permission, scope)
            }
        }
    }
    for (snapshot, denied) in [(false, false), (true, false), (false, true), (true, true)] {
        let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
        let snapshot_id = engine
            .graph()
            .unwrap()
            .revision(revision)
            .unwrap()
            .snapshot_id;
        let policy = engine.policy_authorizer().unwrap();
        let positive_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        if snapshot {
            engine
                .authorize_snapshot_until(&snapshot_id, &principal, &policy, positive_deadline)
                .unwrap();
        } else {
            assert_eq!(
                engine
                    .authorize_revision_until(
                        None,
                        revision,
                        &principal,
                        &policy,
                        positive_deadline
                    )
                    .unwrap(),
                _scope
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        let authorizer = ExpiringDecision {
            policy,
            deadline,
            entered_live: std::cell::Cell::new(false),
            denied,
        };
        let result = if snapshot {
            engine.authorize_snapshot_until(&snapshot_id, &principal, &authorizer, deadline)
        } else {
            engine
                .authorize_revision_until(None, revision, &principal, &authorizer, deadline)
                .map(|_| ())
        };
        assert!(
            authorizer.entered_live.get(),
            "fixture must enter capability before expiry"
        );
        let expected = if denied {
            BusinessError::PermissionDenied
        } else {
            BusinessError::BudgetExceeded
        };
        assert!(
            matches!(&result, Err(EngineError::Business(actual)) if *actual == expected),
            "snapshot={snapshot}, denied={denied}: {result:?}"
        );
    }
}

#[test]
fn trusted_reader_late_capability_preserves_budget_and_explicit_denial() {
    struct SlowCapability {
        policy: diskgraph_core::PolicyAuthorizer,
        initial_deadline: std::time::Instant,
        entered_live: std::cell::Cell<bool>,
        denied: bool,
    }
    impl diskgraph_core::Authorizer for SlowCapability {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.entered_live
                .set(std::time::Instant::now() < self.initial_deadline);
            std::thread::sleep(std::time::Duration::from_millis(300));
            if self.denied {
                diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::Disabled)
            } else {
                self.policy.decide(principal, permission, scope)
            }
        }
    }
    for denied in [false, true] {
        let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
        let authorizer = SlowCapability {
            policy: engine.policy_authorizer().unwrap(),
            initial_deadline: std::time::Instant::now() + std::time::Duration::from_millis(100),
            entered_live: std::cell::Cell::new(false),
            denied,
        };
        let result: Result<(), EngineError> = engine.with_authorized_revision_reader(
            revision,
            &principal,
            &authorizer,
            100,
            |_, _, _| panic!("late initial authorization must not enter consumer"),
        );
        assert!(
            authorizer.entered_live.get(),
            "fixture must enter before conservative original deadline"
        );
        let expected = if denied {
            BusinessError::PermissionDenied
        } else {
            BusinessError::BudgetExceeded
        };
        assert!(matches!(result, Err(EngineError::Business(actual)) if actual == expected));
    }
}

#[test]
fn late_history_side_does_not_hide_the_other_scope_revocation() {
    struct CrossScopeDecision {
        policy: diskgraph_core::PolicyAuthorizer,
        independent: RefCell<diskgraph_store::ControlStore>,
        first: diskgraph_core::ScopeId,
        second: diskgraph_core::ScopeId,
        first_is_slow: bool,
    }
    impl diskgraph_core::Authorizer for CrossScopeDecision {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.first_is_slow && scope == &self.first {
                std::thread::sleep(std::time::Duration::from_millis(80));
            } else if !self.first_is_slow && scope == &self.second {
                self.independent
                    .borrow_mut()
                    .revoke_scope(&self.first)
                    .unwrap();
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let mut failures = Vec::new();
    for first_is_slow in [true, false] {
        let (dir, engine, principal, first, _) = published_authorization_fixture();
        let root = dir.path().join("other-scope");
        std::fs::create_dir(&root).unwrap();
        let second = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let mut independent =
            diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap();
        if first_is_slow {
            independent.revoke_scope(&second).unwrap();
        }
        let authorizer = CrossScopeDecision {
            policy: engine.policy_authorizer().unwrap(),
            independent: RefCell::new(independent),
            first: first.clone(),
            second: second.clone(),
            first_is_slow,
        };
        let control = engine.control_store().unwrap();
        let result = crate::Engine::require_terminal_relations(
            &control,
            &authorizer,
            &principal,
            &[&first, &second],
        );
        if !matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ) {
            failures.push((first_is_slow, result));
        }
    }
    assert!(
        failures.is_empty(),
        "cross-scope denial hidden: {failures:?}"
    );
}

#[test]
fn late_terminal_allow_is_refused_but_actual_revocation_still_wins() {
    struct SlowDecision {
        policy: diskgraph_core::PolicyAuthorizer,
        active: std::cell::Cell<bool>,
        revoke: Option<RefCell<diskgraph_store::ControlStore>>,
        quarantine: Option<rusqlite::Connection>,
        revision: &'static str,
        server: String,
        scope: String,
    }
    impl diskgraph_core::Authorizer for SlowDecision {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.active.get() {
                if let Some(control) = &self.revoke {
                    control.borrow_mut().revoke_scope(scope).unwrap();
                }
                if let Some(connection) = &self.quarantine {
                    connection.execute(
                        "INSERT OR IGNORE INTO revision_access_denials(revision_id,server_id,scope_id,reason) VALUES(?1,?2,?3,'root_identity_unconfirmed')",
                        rusqlite::params![self.revision, self.server, self.scope]).unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    let mut failures = Vec::new();
    for kind in 0..2 {
        for mode in 0..3 {
            let revoked = mode != 0;
            let (dir, engine, principal, _scope, revision) = published_authorization_fixture();
            let slow = SlowDecision {
                policy: engine.policy_authorizer().unwrap(),
                active: std::cell::Cell::new(false),
                revoke: (mode == 1).then(|| {
                    RefCell::new(
                        diskgraph_store::ControlStore::open(
                            &dir.path().join("data/diskgraph-control.sqlite"),
                        )
                        .unwrap(),
                    )
                }),
                quarantine: (mode == 2).then(|| {
                    rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap()
                }),
                revision,
                server: engine.server_id().unwrap().as_str().to_owned(),
                scope: _scope.as_str().to_owned(),
            };
            let encoded = std::cell::Cell::new(false);
            let consume = || {
                slow.active.set(true);
                Ok(())
            };
            let finish = |_: &mut (), _: bool| {
                encoded.set(true);
                Ok(())
            };
            let budget = QueryBudget::default();
            let deadline = query_deadline(budget).unwrap();
            let result = if kind == 0 {
                engine.with_relation_reader_until(
                    revision,
                    &principal,
                    &slow,
                    deadline,
                    budget,
                    None,
                    |_, _, _| consume(),
                    finish,
                )
            } else {
                engine.with_history_readers_until(
                    revision,
                    revision,
                    &principal,
                    &slow,
                    deadline,
                    budget,
                    |_, _, _, _, _, _| consume(),
                    finish,
                )
            };
            let expected = if revoked {
                BusinessError::PermissionDenied
            } else {
                BusinessError::BudgetExceeded
            };
            if !matches!(result, Err(EngineError::Business(error)) if error == expected)
                || encoded.get()
            {
                failures.push((kind, revoked, result, encoded.get()));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "late terminal decision escaped: {failures:?}"
    );
}

#[test]
fn reader_capability_callback_does_not_inherit_sql_deadline_handler() {
    struct Probe<'a> {
        control: &'a diskgraph_store::ControlStore,
        sql_succeeded: std::cell::Cell<bool>,
    }
    impl diskgraph_core::Authorizer for Probe<'_> {
        fn decide(
            &self,
            _: &PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            std::thread::sleep(std::time::Duration::from_millis(150));
            self.sql_succeeded
                .set(self.control.existing_server_id().is_ok());
            diskgraph_core::Decision::Allowed
        }
    }
    let (_dir, engine, principal, scope, _) = published_authorization_fixture();
    let control = engine.control_store().unwrap();
    let probe = Probe {
        control: &control,
        sql_succeeded: std::cell::Cell::new(false),
    };
    let result = crate::Engine::require_reader_capability_until(
        &control,
        &probe,
        &principal,
        &scope,
        None,
        std::time::Instant::now() + std::time::Duration::from_millis(100),
    );
    assert!(
        probe.sql_succeeded.get(),
        "host callback inherited expired SQL handler"
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn trusted_reader_terminal_late_allow_is_refused_and_revocation_wins() {
    struct Late<'a> {
        policy: diskgraph_core::PolicyAuthorizer,
        active: &'a std::cell::Cell<bool>,
        revoke: Option<RefCell<diskgraph_store::ControlStore>>,
    }
    impl diskgraph_core::Authorizer for Late<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.active.get() {
                if let Some(control) = &self.revoke {
                    control.borrow_mut().revoke_scope(scope).unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for revoked in [false, true] {
        let (dir, engine, principal, _scope, revision) = published_authorization_fixture();
        let active = std::cell::Cell::new(false);
        let authorizer = Late {
            policy: engine.policy_authorizer().unwrap(),
            active: &active,
            revoke: revoked.then(|| {
                RefCell::new(
                    diskgraph_store::ControlStore::open(
                        &dir.path().join("data/diskgraph-control.sqlite"),
                    )
                    .unwrap(),
                )
            }),
        };
        let result = engine.with_authorized_revision_reader(
            revision,
            &principal,
            &authorizer,
            1000,
            |_, _, _| {
                active.set(true);
                Ok(())
            },
        );
        let expected = if revoked {
            BusinessError::PermissionDenied
        } else {
            BusinessError::BudgetExceeded
        };
        assert!(
            matches!(result, Err(EngineError::Business(actual)) if actual == expected),
            "terminal result: {result:?}"
        );
    }
}

#[test]
fn latest_resolution_reads_existing_identity_and_never_repairs_missing_server() {
    let (dir, engine, _principal, scope, revision) = published_authorization_fixture();
    let deadline = || std::time::Instant::now() + std::time::Duration::from_secs(1);
    assert_eq!(
        engine
            .latest_revision_until(&scope, deadline())
            .unwrap()
            .as_deref(),
        Some(revision)
    );
    let independent =
        rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    independent
        .execute("DELETE FROM server WHERE id=1", [])
        .unwrap();
    assert!(matches!(
        engine.latest_revision_until(&scope, deadline()),
        Err(EngineError::Store(
            diskgraph_store::StoreError::InvalidGraph(_)
        ))
    ));
    let count: i64 = independent
        .query_row("SELECT COUNT(*) FROM server", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "bounded read must not mint a replacement identity"
    );
}
