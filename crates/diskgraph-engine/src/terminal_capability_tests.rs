//! 关系与历史的迟到能力、跨侧撤权与隔离优先级回归。
use crate::EngineError;
use crate::relation_request_tests::published_authorization_fixture;
use diskgraph_core::{BusinessError, Permission, PrincipalId, QueryBudget, query_deadline};
use std::cell::RefCell;

#[test]
fn expired_relation_history_and_envelope_capability_is_denied() {
    struct Expired;
    impl diskgraph_core::Authorizer for Expired {
        fn decide(
            &self,
            _: &PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            diskgraph_core::Decision::Allowed
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(0)
        }
    }
    for kind in 0..3 {
        let (_dir, engine, principal, _, revision) = published_authorization_fixture();
        let budget = QueryBudget::default();
        let deadline = query_deadline(budget).unwrap();
        let result = match kind {
            0 => engine.with_relation_reader_until(
                revision,
                &principal,
                &Expired,
                deadline,
                budget,
                None,
                |_, _, _| panic!("expired relation entered consumer"),
                |_: &mut (), _| Ok(()),
            ),
            1 => engine.with_history_readers_until(
                revision,
                revision,
                &principal,
                &Expired,
                deadline,
                budget,
                |_, _, _, _, _, _| panic!("expired history entered consumer"),
                |_: &mut (), _| Ok(()),
            ),
            _ => engine
                .finalize_revisions_read_until(
                    &[revision, revision],
                    &principal,
                    &Expired,
                    deadline,
                )
                .map(|_| ()),
        };
        assert!(
            matches!(
                result,
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "expired kind {kind}: {result:?}"
        );
    }
}

#[test]
fn envelope_expiry_during_callback_stops_remaining_observations() {
    struct Expiring {
        expiry: u64,
        calls: std::cell::Cell<u32>,
    }
    impl diskgraph_core::Authorizer for Expiring {
        fn decide(
            &self,
            _: &PrincipalId,
            _: &Permission,
            _: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.calls.set(self.calls.get() + 1);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            assert!(
                now.as_secs() < self.expiry,
                "callback fixture entered too late"
            );
            std::thread::sleep(
                std::time::Duration::from_secs(self.expiry).saturating_sub(now)
                    + std::time::Duration::from_millis(20),
            );
            diskgraph_core::Decision::Allowed
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    let (_dir, engine, principal, _, revision) = published_authorization_fixture();
    let authorizer = Expiring {
        expiry: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2,
        calls: std::cell::Cell::new(0),
    };
    let deadline = query_deadline(QueryBudget::default()).unwrap();
    let result = engine.finalize_revisions_read_until(
        &[revision, revision],
        &principal,
        &authorizer,
        deadline,
    );
    assert_eq!(authorizer.calls.get(), 1);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired envelope callback: {result:?}"
    );
}

#[test]
fn relation_and_history_preserve_original_expiry_across_consumer_and_encoding() {
    struct Fixed {
        policy: diskgraph_core::PolicyAuthorizer,
        expiry: u64,
        getters: std::cell::Cell<u32>,
    }
    impl diskgraph_core::Authorizer for Fixed {
        fn decide(
            &self,
            p: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.policy.decide(p, permission, scope)
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            let first = self.getters.get() == 0;
            self.getters.set(self.getters.get() + 1);
            first.then_some(self.expiry)
        }
    }
    for history in [false, true] {
        for during_encoding in [false, true] {
            let (_dir, engine, principal, _, revision) = published_authorization_fixture();
            let policy = engine.policy_authorizer().unwrap();
            let expiry = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 2;
            let authorizer = Fixed {
                policy,
                expiry,
                getters: std::cell::Cell::new(0),
            };
            let entered = std::cell::Cell::new(false);
            let encoded = std::cell::Cell::new(false);
            let wait_expiry = || {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap();
                assert!(
                    now.as_secs() < expiry,
                    "fixture must enter while capability is live"
                );
                std::thread::sleep(
                    std::time::Duration::from_secs(expiry).saturating_sub(now)
                        + std::time::Duration::from_millis(20),
                );
            };
            let consume = || {
                entered.set(true);
                if !during_encoding {
                    wait_expiry();
                }
                Ok(())
            };
            let finish = |_: &mut (), _: bool| {
                encoded.set(true);
                if during_encoding {
                    wait_expiry();
                }
                Ok(())
            };
            let budget = QueryBudget::default();
            let deadline = query_deadline(budget).unwrap();
            let result = if history {
                engine.with_history_readers_until(
                    revision,
                    revision,
                    &principal,
                    &authorizer,
                    deadline,
                    budget,
                    |_, _, _, _, _, _| consume(),
                    finish,
                )
            } else {
                engine.with_relation_reader_until(
                    revision,
                    &principal,
                    &authorizer,
                    deadline,
                    budget,
                    None,
                    |_, _, _| consume(),
                    finish,
                )
            };
            assert!(entered.get());
            assert_eq!(encoded.get(), during_encoding);
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "original expiry history={history} encode={during_encoding}: {result:?}"
            );
        }
    }
}

#[test]
fn relation_and_envelope_callbacks_can_reenter_control_and_revoke() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        active: std::cell::Cell<bool>,
        reentered: std::cell::Cell<bool>,
        revoke: bool,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.active.get() {
                let mut control = self
                    .engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
                    .expect("relation or envelope callback held control");
                self.reentered.set(true);
                if self.revoke {
                    control.revoke_grant(principal, permission, scope).unwrap();
                }
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for envelope in [false, true] {
        for revoke in [false, true] {
            let (_dir, engine, principal, _, revision) = published_authorization_fixture();
            let authorizer = Callback {
                engine: &engine,
                policy: engine.policy_authorizer().unwrap(),
                active: std::cell::Cell::new(envelope),
                reentered: std::cell::Cell::new(false),
                revoke,
            };
            let budget = QueryBudget::default();
            let deadline = query_deadline(budget).unwrap();
            let encoded = std::cell::Cell::new(false);
            let result = if envelope {
                engine
                    .finalize_revisions_read_until(
                        &[revision, revision],
                        &principal,
                        &authorizer,
                        deadline,
                    )
                    .map(|_| ())
            } else {
                engine.with_relation_reader_until(
                    revision,
                    &principal,
                    &authorizer,
                    deadline,
                    budget,
                    None,
                    |_, _, _| {
                        authorizer.active.set(true);
                        Ok(())
                    },
                    |_, _| {
                        encoded.set(true);
                        Ok(())
                    },
                )
            };
            assert!(authorizer.reentered.get());
            if revoke {
                assert!(
                    matches!(
                        result,
                        Err(EngineError::Business(BusinessError::PermissionDenied))
                    ),
                    "revoked terminal capability: {result:?}"
                );
                assert!(!encoded.get());
            } else {
                result.unwrap();
                assert!(envelope || encoded.get());
            }
        }
    }
}

#[test]
fn history_terminal_callbacks_can_reenter_and_revoke_the_other_scope() {
    struct Callback<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        first: diskgraph_core::ScopeId,
        second: diskgraph_core::ScopeId,
        active: std::cell::Cell<bool>,
        reentered: std::cell::Cell<bool>,
        revoke: bool,
    }
    impl diskgraph_core::Authorizer for Callback<'_> {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            if self.active.get() {
                let mut control = self
                    .engine
                    .control_until(std::time::Instant::now() + std::time::Duration::from_millis(50))
                    .expect("history terminal callback held control");
                self.reentered.set(true);
                if self.revoke && scope == &self.second {
                    control
                        .revoke_grant(principal, permission, &self.first)
                        .unwrap();
                }
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for revoke in [false, true] {
        let (dir, engine, principal, first, revision) = published_authorization_fixture();
        let root = dir.path().join("second-history-scope");
        std::fs::create_dir(&root).unwrap();
        let second = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let second_revision = "unlocked-second-history-revision";
        let server = engine.server_id().unwrap();
        {
            let mut store = engine.graph().unwrap();
            let mut graph = store.load_revision(revision).unwrap();
            graph.snapshot.id = "unlocked-second-history-snapshot".into();
            store
                .append_staging_nodes("unlocked-second-history-job", &graph.nodes)
                .unwrap();
            store
                .publish_revision_owned(
                    "unlocked-second-history-job",
                    &graph,
                    second_revision,
                    2,
                    Some((server.as_str(), second.as_str())),
                )
                .unwrap();
        }
        let authorizer = Callback {
            engine: &engine,
            policy: engine.policy_authorizer().unwrap(),
            first,
            second,
            active: std::cell::Cell::new(false),
            reentered: std::cell::Cell::new(false),
            revoke,
        };
        let encoded = std::cell::Cell::new(false);
        let budget = QueryBudget::default();
        let result = engine.with_history_readers_until(
            revision,
            second_revision,
            &principal,
            &authorizer,
            query_deadline(budget).unwrap(),
            budget,
            |_, _, _, _, _, _| {
                authorizer.active.set(true);
                Ok(())
            },
            |_, _| {
                encoded.set(true);
                Ok(())
            },
        );
        assert!(authorizer.reentered.get());
        if revoke {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "other-side withdrawal: {result:?}"
            );
            assert!(!encoded.get());
        } else {
            result.unwrap();
            assert!(encoded.get());
        }
    }
}

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
                let ownership = reader
                    .revision_ownership_with_budget(revision, &mut reads)
                    .unwrap();
                engine
                    .authorize_revision_owner_until(
                        ownership,
                        None,
                        &principal,
                        &policy,
                        reads.deadline(),
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
        let result =
            engine.require_terminal_relations(&authorizer, &principal, &[&first, &second], None);
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
    let (dir, engine, principal, scope, _) = published_authorization_fixture();
    let control =
        diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
            .unwrap();
    let probe = Probe {
        control: &control,
        sql_succeeded: std::cell::Cell::new(false),
    };
    let result = engine.require_reader_capability_until(
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
