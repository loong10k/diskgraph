//! 关系与历史的迟到能力、跨侧撤权与隔离优先级回归。
use crate::EngineError;
use crate::relation_request_tests::published_authorization_fixture;
use diskgraph_core::{BusinessError, Permission, PrincipalId, QueryBudget, query_deadline};
use std::cell::RefCell;

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
