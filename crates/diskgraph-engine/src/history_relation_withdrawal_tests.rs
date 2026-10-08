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
            let withdrawal_committed = std::cell::Cell::new(false);
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
                        withdrawal_committed.set(true);
                    }
                    Ok(())
                },
                |_, _| {
                    if during_encode {
                        revoke_and_restore(&engine, &actor, scope);
                        withdrawal_committed.set(true);
                    }
                    Ok(())
                },
            );
            // 期限在目标撤权之前耗尽属于前置失败，不能冒充连续撤权逻辑的反例或通过。
            assert!(
                withdrawal_committed.get(),
                "target withdrawal not reached: right_side={right_side}, during_encode={during_encode}, actual={result:?}"
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

// 仅测试：在已建立的原窗口内制造确定性过期，不改生产时钟或查询额度。
thread_local! {
    static EXPIRE_TERMINAL_SQL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 在本线程指定的原 SQL 观察窗口中制造过期；参数为原期限，返回后生产逻辑继续拒绝。
pub(super) fn before_terminal_sql(deadline: std::time::Instant) {
    if EXPIRE_TERMINAL_SQL.with(|expire| expire.replace(false)) {
        std::thread::sleep(
            deadline.saturating_duration_since(std::time::Instant::now())
                + std::time::Duration::from_millis(2),
        );
    }
}

#[test]
fn committed_withdrawal_survives_terminal_sql_expiry() {
    terminal_sql_expiry_preserves_known_denial(true);
}

#[test]
fn terminal_sql_expiry_without_withdrawal_remains_budget_exceeded() {
    terminal_sql_expiry_preserves_known_denial(false);
}

fn terminal_sql_expiry_preserves_known_denial(withdraw: bool) {
    let mut failures = Vec::new();
    for history in [false, true] {
        let (_dir, engine, actor, scope, revision) =
            crate::relation_request_tests::published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        let native = engine
            .control_store()
            .unwrap()
            .watch_authorization_withdrawal(&actor, &scope, &Permission::MetadataRead)
            .unwrap()
            .is_some();
        #[cfg(windows)]
        assert!(
            native,
            "Windows fixture must exercise actual native withdrawal identity"
        );
        let target = if history { 2 } else { 1 };
        let authority = CallbackWithdrawal {
            engine: &engine,
            policy: &policy,
            scope: &scope,
            calls: std::cell::Cell::new(0),
            target: if withdraw { target } else { usize::MAX },
        };
        let consumed = std::cell::Cell::new(false);
        let encoded = std::cell::Cell::new(false);
        let consume = || {
            consumed.set(true);
            EXPIRE_TERMINAL_SQL.with(|expire| expire.set(true));
            Ok(())
        };
        let finish = |_: &mut (), _: bool| {
            encoded.set(true);
            Ok(())
        };
        let budget = diskgraph_core::QueryBudget::default();
        let deadline = diskgraph_core::query_deadline(budget).unwrap();
        let result = if history {
            engine.with_history_readers_until(
                revision,
                revision,
                &actor,
                &authority,
                deadline,
                budget,
                |_, _, _, _, _, _| consume(),
                finish,
            )
        } else {
            engine.with_relation_reader_until(
                revision,
                &actor,
                &authority,
                deadline,
                budget,
                None,
                |_, _, _| consume(),
                finish,
            )
        };
        assert!(consumed.get());
        assert!(
            !encoded.get(),
            "failed authorization must not enter encoding"
        );
        assert!(
            !EXPIRE_TERMINAL_SQL.with(std::cell::Cell::get),
            "original SQL window was not exercised"
        );
        assert!(
            authority.calls.get() > target,
            "terminal callback must actually execute"
        );
        let expected = if withdraw && native {
            BusinessError::PermissionDenied
        } else {
            BusinessError::BudgetExceeded
        };
        if !matches!(&result, Err(EngineError::Business(error)) if *error == expected) {
            failures.push(format!(
                "history={history}, native={native}: {result:?}, expected {expected:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "terminal withdrawal priority: {failures:?}"
    );
}
