#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{
    BusinessError, Permission, PrincipalId, QueryBudget, TruncationReason, query_deadline,
};
use std::cell::RefCell;
use std::sync::{Arc, mpsc};
use std::time::Instant;

// 仅测试的读后一次性同步；不进入生产模块或 Engine 状态。
type AfterRead = Box<dyn FnOnce(Instant)>;
thread_local! {
    static AFTER_READ: RefCell<Option<AfterRead>> = RefCell::new(None);
}

pub(super) fn after_read(deadline: Instant) {
    if let Some(callback) = AFTER_READ.with(|slot| slot.borrow_mut().take()) {
        callback(deadline);
    }
}

fn fixture() -> (
    tempfile::TempDir,
    Arc<Engine>,
    PrincipalId,
    diskgraph_core::ScopeId,
    diskgraph_core::ScopeId,
    String,
) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["root", "other"] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    std::fs::write(dir.path().join("root/file"), b"payload").unwrap();
    let engine = Arc::new(
        Engine::open(EngineConfig {
            data_dir: dir.path().join("data"),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let principal = PrincipalId::new("terminal-reader").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(
            &dir.path().join("root"),
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let other = engine
        .register_scope(
            &dir.path().join("other"),
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let job = engine
        .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "terminal-reader").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let snapshot = engine
        .revision_reader()
        .unwrap()
        .revision(&revision)
        .unwrap()
        .snapshot_id;
    let run = diskgraph_core::CollectorRun {
        run_id: "fixture".into(),
        snapshot_id: snapshot.clone(),
        collector_id: "terminal-authorization-fixture".into(),
        collector_version: 1,
        rule_version: 1,
        observed_at_unix_ms: 1,
        coverage_complete: true,
        errors: vec![],
    };
    let entity = diskgraph_core::Entity {
        entity_id: "fixture".into(),
        kind: diskgraph_core::EntityKind::Resource,
        identity: "fixture".into(),
        display: "fixture".into(),
        source_run_id: run.run_id.clone(),
    };
    let mut store =
        diskgraph_store::SqliteSnapshotStore::open(&dir.path().join("data/diskgraph.sqlite"))
            .unwrap();
    let owner = store.revision_ownership(&revision).unwrap().unwrap();
    let next_revision = format!("{revision}-fixture");
    let batch = diskgraph_core::CollectorBatch {
        run: run.clone(),
        entities: vec![entity],
        evidence: vec![],
        edges: vec![],
    };
    store
        .publish_collector_revision(
            &revision,
            &next_revision,
            2,
            (&owner.0, &owner.1),
            &batch,
            &[(&run.run_id, "active")],
        )
        .unwrap();
    let revision = next_revision;
    (dir, engine, principal, scope, other, revision)
}

fn query(
    engine: &Engine,
    principal: &PrincipalId,
    policy: &dyn diskgraph_core::Authorizer,
    revision: &str,
    kind: u8,
    budget: QueryBudget,
) -> Result<(), EngineError> {
    let deadline = query_deadline(budget)?;
    match kind {
        0 => engine
            .review_candidates_until(revision, 0, budget, principal, policy, deadline)
            .map(|_| ()),
        1 => engine
            .revision_impact_until(revision, "fixture", budget, principal, policy, deadline)
            .map(|_| ()),
        2 => engine
            .related_bounded_until(
                revision, "fixture", None, true, None, 1, budget, principal, policy, deadline,
            )
            .map(|_| ()),
        3 => engine
            .explain_bounded_until(
                revision, "fixture", None, 1, budget, principal, policy, deadline,
            )
            .map(|_| ()),
        _ => panic!("invalid fixture query"),
    }
}

#[test]
fn all_relation_answers_refuse_actual_scope_revoked_after_data_read() {
    for kind in 0..4 {
        let (_dir, engine, principal, scope, _other, revision) = fixture();
        let policy = engine.policy_authorizer().unwrap();
        let callback_engine = engine.clone();
        let callback_principal = principal.clone();
        let callback_policy = policy.clone();
        AFTER_READ.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |_| {
                callback_engine
                    .revoke_scope(&scope, &callback_principal, &callback_policy)
                    .unwrap();
            }))
        });
        assert!(
            matches!(
                query(
                    &engine,
                    &principal,
                    &policy,
                    &revision,
                    kind,
                    QueryBudget::default()
                ),
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "query {kind} returned a revoked scope response"
        );
        assert!(
            AFTER_READ.with(|slot| slot.borrow().is_none()),
            "terminal synchronization was not reached"
        );
    }
}

#[test]
fn all_relation_answers_refuse_one_grant_revoked_while_another_scope_remains() {
    for kind in 0..4 {
        let (_dir, engine, principal, scope, other, revision) = fixture();
        let policy = engine.policy_authorizer().unwrap();
        let callback_engine = engine.clone();
        let callback_principal = principal.clone();
        AFTER_READ.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |_| {
                let mut control = callback_engine.control_store().unwrap();
                control
                    .revoke_grant(&callback_principal, &Permission::MetadataRead, &scope)
                    .unwrap();
                assert_eq!(
                    control
                        .live_permission(&callback_principal, &Permission::MetadataRead, &other)
                        .unwrap(),
                    Some(true)
                );
            }))
        });
        assert!(
            matches!(
                query(
                    &engine,
                    &principal,
                    &policy,
                    &revision,
                    kind,
                    QueryBudget::default()
                ),
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "query {kind} returned a individually revoked response"
        );
        assert!(AFTER_READ.with(|slot| slot.borrow().is_none()));
    }
}

#[test]
fn actual_terminal_control_guard_wait_cannot_be_a_complete_empty_result() {
    let (_dir, engine, principal, _scope, _other, revision) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let worker_engine = engine.clone();
    let worker = std::thread::spawn(move || {
        AFTER_READ.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |deadline| {
                ready_tx.send(deadline).unwrap();
                resume_rx.recv().unwrap();
            }))
        });
        worker_engine
            .review_candidates(&revision, 0, QueryBudget::default(), &principal, &policy)
            .unwrap()
    });
    let deadline = ready_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let guard = engine.control_store().unwrap();
    resume_tx.send(()).unwrap();
    // 同步点之后确实由另一线程持有真实 control guard，直到原绝对期限过后才释放。
    std::thread::sleep(
        deadline.saturating_duration_since(Instant::now()) + std::time::Duration::from_millis(20),
    );
    drop(guard);
    let answer = worker.join().unwrap();
    assert!(!answer.complete);
    assert_eq!(answer.truncated, Some(TruncationReason::Deadline));
}

#[test]
fn a_post_authorization_budget_failure_still_requires_live_terminal_authorization() {
    for kind in 0..4 {
        let (_dir, engine, principal, scope, _other, revision) = fixture();
        let policy = engine.policy_authorizer().unwrap();
        let owner = engine
            .revision_reader()
            .unwrap()
            .revision_ownership(&revision)
            .unwrap()
            .unwrap();
        // 真实归属必须先能准入；仅够归属的余额在目标准备阶段耗尽。
        // 1 字节预算无法建立授权上下文，由下方最小额度测试单独覆盖。
        let owner_bytes = owner.0.len() + owner.1.len();
        let callback_engine = engine.clone();
        let callback_principal = principal.clone();
        let callback_policy = policy.clone();
        AFTER_READ.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move |_| {
                callback_engine
                    .revoke_scope(&scope, &callback_principal, &callback_policy)
                    .unwrap();
            }));
        });
        assert!(
            matches!(
                query(
                    &engine,
                    &principal,
                    &policy,
                    &revision,
                    kind,
                    QueryBudget {
                        max_response_bytes: owner_bytes,
                        ..QueryBudget::default()
                    }
                ),
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "query {kind} reported a budget error before checking actual revocation"
        );
        assert!(AFTER_READ.with(|slot| slot.borrow().is_none()));
    }
}

#[test]
fn the_minimum_diagnostic_must_fit_its_own_actual_json_budget() {
    let (_dir, engine, principal, _scope, _other, revision) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    for kind in 0..4 {
        assert!(
            matches!(
                query(
                    &engine,
                    &principal,
                    &policy,
                    &revision,
                    kind,
                    QueryBudget {
                        max_response_bytes: 1,
                        ..QueryBudget::default()
                    }
                ),
                Err(EngineError::Business(BusinessError::BudgetExceeded))
                    | Err(EngineError::Store(
                        diskgraph_store::StoreError::BudgetExceeded
                    ))
            ),
            "query {kind} returned an unrepresentable diagnostic"
        );
    }
}

#[test]
fn an_independent_control_connection_can_revoke_the_scope_during_finish() {
    let (dir, engine, principal, scope, _other, revision) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let path = dir.path().join("data/diskgraph-control.sqlite");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM policy", [])
        .unwrap();
    assert_eq!(
        engine.control_store().unwrap().policy_state().unwrap(),
        None
    );
    let mut independent = diskgraph_store::ControlStore::open(&path).unwrap();
    let budget = QueryBudget::default();
    let result = engine.with_relation_reader_until(
        &revision,
        &principal,
        &policy,
        query_deadline(budget).unwrap(),
        budget,
        None,
        |_, _, _| Ok(serde_json::json!({"complete":true})),
        |answer, _| {
            assert!(
                diskgraph_core::measure_json_bounded(answer, budget.max_response_bytes)
                    .unwrap()
                    .is_some()
            );
            // 原生第二连接完成真实撤销；不得回入当前 Engine 的 control Mutex。
            independent.revoke_scope(&scope).unwrap();
            Ok(())
        },
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "finish returned data after an independent connection revoked its actual scope"
    );
}

#[test]
fn final_envelope_authorization_refuses_revocation_during_its_authorizer_call() {
    struct RevokeDuringDecision {
        policy: diskgraph_core::PolicyAuthorizer,
        independent: RefCell<diskgraph_store::ControlStore>,
    }
    impl diskgraph_core::Authorizer for RevokeDuringDecision {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.independent.borrow_mut().revoke_scope(scope).unwrap();
            self.policy.decide(principal, permission, scope)
        }
    }
    let (dir, engine, principal, _scope, _other, revision) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let path = dir.path().join("data/diskgraph-control.sqlite");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM policy", [])
        .unwrap();
    assert_eq!(
        engine.control_store().unwrap().policy_state().unwrap(),
        None
    );
    let authorizer = RevokeDuringDecision {
        policy,
        independent: RefCell::new(diskgraph_store::ControlStore::open(&path).unwrap()),
    };
    let result = engine.finalize_revision_read_until(
        &revision,
        &principal,
        &authorizer,
        query_deadline(QueryBudget::default()).unwrap(),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "the adapter finalizer passed a scope revoked during its authorizer call"
    );
}

#[test]
fn encoded_budget_failure_still_checks_the_real_terminal_revocation() {
    let (dir, engine, principal, scope, _other, revision) = fixture();
    let policy = engine.policy_authorizer().unwrap();
    let mut independent =
        diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
            .unwrap();
    let budget = QueryBudget::default();
    let result = engine.with_relation_reader_until(
        &revision,
        &principal,
        &policy,
        query_deadline(budget).unwrap(),
        budget,
        None,
        |_, _, _| Ok(serde_json::json!({"complete":true})),
        |_, _| {
            independent.revoke_scope(&scope).unwrap();
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        },
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "encoded error hid the actual terminal denial: {result:?}"
    );
}
