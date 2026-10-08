//! 历史与关系的读取、编码期间真实撤权恢复回归；来源：DiskGraph 原生安全契约。
use crate::{Engine, EngineError};
use diskgraph_core::{BusinessError, Permission, PrincipalId, ScopeId};

fn revoke_and_restore(engine: &Engine, principal: &PrincipalId, scope: &ScopeId) {
    let mut control = engine.control_store().unwrap();
    control
        .revoke_grant(principal, &Permission::MetadataRead, scope)
        .unwrap();
    let policy_version = control.policy_version().unwrap();
    control
        .upsert_grant(&diskgraph_core::Grant {
            principal: principal.clone(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version,
        })
        .unwrap();
}

fn assert_withdrawn(result: Result<(), EngineError>, native: bool) {
    let expected = if native {
        BusinessError::PermissionDenied
    } else {
        BusinessError::Conflict
    };
    assert!(
        matches!(result, Err(EngineError::Business(error)) if error == expected),
        "authorization churn returned {result:?}, expected {expected:?}"
    );
}

#[test]
fn history_request_remembers_revocation_during_read_and_encode() {
    for during_encode in [false, true] {
        let (_dir, engine, actor, scope, revision) =
            crate::relation_request_tests::published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        let native = engine
            .control_store()
            .unwrap()
            .watch_authorization_withdrawal(&actor, &scope, &Permission::MetadataRead)
            .unwrap()
            .is_some();
        let budget = diskgraph_core::QueryBudget {
            deadline_ms: 1000,
            ..diskgraph_core::QueryBudget::default()
        };
        let result = engine.with_history_readers_until(
            revision,
            revision,
            &actor,
            &policy,
            diskgraph_core::query_deadline(budget).unwrap(),
            budget,
            |_, _, _, _, _, _| {
                if !during_encode {
                    revoke_and_restore(&engine, &actor, &scope);
                }
                Ok(())
            },
            |_, _| {
                if during_encode {
                    revoke_and_restore(&engine, &actor, &scope);
                }
                Ok(())
            },
        );
        assert_withdrawn(result, native);
    }
}

#[test]
fn relation_request_remembers_revocation_during_read_and_encode() {
    for during_encode in [false, true] {
        let (_dir, engine, actor, scope, revision) =
            crate::relation_request_tests::published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        let native = engine
            .control_store()
            .unwrap()
            .watch_authorization_withdrawal(&actor, &scope, &Permission::MetadataRead)
            .unwrap()
            .is_some();
        let budget = diskgraph_core::QueryBudget {
            deadline_ms: 1000,
            ..diskgraph_core::QueryBudget::default()
        };
        let result = engine.with_relation_reader_until(
            revision,
            &actor,
            &policy,
            diskgraph_core::query_deadline(budget).unwrap(),
            budget,
            None,
            |_, _, _| {
                if !during_encode {
                    revoke_and_restore(&engine, &actor, &scope);
                }
                Ok(())
            },
            |_, _| {
                if during_encode {
                    revoke_and_restore(&engine, &actor, &scope);
                }
                Ok(())
            },
        );
        assert_withdrawn(result, native);
    }
}

#[test]
fn history_request_remembers_each_distinct_scope_through_encoding() {
    for right_side in [false, true] {
        for during_encode in [false, true] {
            let (dir, engine, actor, left_scope, revision) =
                crate::relation_request_tests::published_authorization_fixture();
            let root = dir.path().join("second-root");
            std::fs::create_dir(&root).unwrap();
            let right_scope = engine
                .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
                .unwrap();
            let server = engine.server_id().unwrap();
            {
                let mut store = engine.graph().unwrap();
                let mut graph = store.load_revision(revision).unwrap();
                graph.snapshot.id = "withdrawal-second-snapshot".into();
                graph.snapshot.root =
                    serde_json::from_value(serde_json::json!({"type":"native_path","value":root}))
                        .unwrap();
                graph.nodes[0].locator = graph.snapshot.root.clone();
                store
                    .append_staging_nodes("withdrawal-second-job", &graph.nodes)
                    .unwrap();
                store
                    .publish_revision_owned(
                        "withdrawal-second-job",
                        &graph,
                        "withdrawal-second-revision",
                        2,
                        Some((server.as_str(), right_scope.as_str())),
                    )
                    .unwrap();
            }
            let scope = if right_side {
                &right_scope
            } else {
                &left_scope
            };
            let native = engine
                .control_store()
                .unwrap()
                .watch_authorization_withdrawal(&actor, scope, &Permission::MetadataRead)
                .unwrap()
                .is_some();
            let policy = engine.policy_authorizer().unwrap();
            let budget = diskgraph_core::QueryBudget {
                deadline_ms: 1000,
                ..diskgraph_core::QueryBudget::default()
            };
            let result = engine.with_history_readers_until(
                revision,
                "withdrawal-second-revision",
                &actor,
                &policy,
                diskgraph_core::query_deadline(budget).unwrap(),
                budget,
                |_, _, _, _, _, same_scope| {
                    assert!(!same_scope);
                    if !during_encode {
                        revoke_and_restore(&engine, &actor, scope);
                    }
                    Ok(())
                },
                |_, _| {
                    if during_encode {
                        revoke_and_restore(&engine, &actor, scope);
                    }
                    Ok(())
                },
            );
            assert_withdrawn(result, native);
        }
    }
}

/// 在指定锁外能力观察实际撤权恢复，验证请求见证跨越全部能力阶段。
/// 来源：DiskGraph 原生历史与关系授权回归。
struct CallbackWithdrawal<'a> {
    engine: &'a Engine,
    policy: &'a dyn diskgraph_core::Authorizer,
    scope: &'a ScopeId,
    calls: std::cell::Cell<usize>,
    target: usize,
}

impl diskgraph_core::Authorizer for CallbackWithdrawal<'_> {
    fn decide(
        &self,
        actor: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> diskgraph_core::Decision {
        assert!(self.engine.try_control_store().unwrap().is_some());
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == self.target {
            revoke_and_restore(self.engine, actor, self.scope);
        }
        self.policy.decide(actor, permission, scope)
    }
}

#[test]
fn history_and_relation_remember_withdrawal_in_capability_observations() {
    for history in [false, true] {
        let count = if history { 6 } else { 3 };
        for target in 0..count {
            let (_dir, engine, actor, scope, revision) =
                crate::relation_request_tests::published_authorization_fixture();
            let policy = engine.policy_authorizer().unwrap();
            let native = engine
                .control_store()
                .unwrap()
                .watch_authorization_withdrawal(&actor, &scope, &Permission::MetadataRead)
                .unwrap()
                .is_some();
            let authority = CallbackWithdrawal {
                engine: &engine,
                policy: &policy,
                scope: &scope,
                calls: std::cell::Cell::new(0),
                target,
            };
            let budget = diskgraph_core::QueryBudget {
                deadline_ms: 1000,
                ..diskgraph_core::QueryBudget::default()
            };
            let deadline = diskgraph_core::query_deadline(budget).unwrap();
            let result = if history {
                engine.with_history_readers_until(
                    revision,
                    revision,
                    &actor,
                    &authority,
                    deadline,
                    budget,
                    |_, _, _, _, _, _| Ok(()),
                    |_, _| Ok(()),
                )
            } else {
                engine.with_relation_reader_until(
                    revision,
                    &actor,
                    &authority,
                    deadline,
                    budget,
                    None,
                    |_, _, _| Ok(()),
                    |_, _| Ok(()),
                )
            };
            assert!(
                authority.calls.get() > target,
                "selected callback did not execute"
            );
            assert_withdrawn(result, native);
        }
    }
}
