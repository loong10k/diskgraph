#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{BusinessError, Permission, PrincipalId, QueryBudget, query_deadline};
use std::cell::RefCell;
use std::sync::{Arc, mpsc};
use std::time::Instant;

// 仅测试的读后一次性同步；不进入生产模块或 Engine 状态。
type AfterRead = Box<dyn FnOnce(Instant)>;
thread_local! {
    static AFTER_READ: RefCell<Option<AfterRead>> = RefCell::new(None);
    static TERMINAL_READER_OPENS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn terminal_reader_opened() {
    TERMINAL_READER_OPENS.with(|count| count.set(count.get() + 1));
}

#[test]
fn history_terminal_ownership_opens_one_fresh_reader_per_observation_round() {
    let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    let budget = QueryBudget::default();
    TERMINAL_READER_OPENS.with(|count| count.set(0));
    engine
        .with_history_readers_until(
            revision,
            revision,
            &principal,
            &policy,
            query_deadline(budget).unwrap(),
            budget,
            |_, _, _, _, _, _| Ok(()),
            |_, _| Ok(()),
        )
        .unwrap();
    // 编码前后各有一轮新鲜连接，双侧不重复打开；消费者的原连接不计入此指标。
    assert_eq!(TERMINAL_READER_OPENS.with(std::cell::Cell::get), 2);
}

#[test]
fn shared_terminal_reader_checks_the_second_distinct_revision_ownership() {
    let (_dir, engine, _principal, scope, revision) = published_authorization_fixture();
    let second = "second-terminal-revision";
    let server = engine.server_id().unwrap();
    {
        let mut store = engine.graph().unwrap();
        let mut graph = store.load_revision(revision).unwrap();
        graph.snapshot.id = "second-terminal-snapshot".into();
        store
            .append_staging_nodes("second-terminal-job", &graph.nodes)
            .unwrap();
        store
            .publish_revision_owned(
                "second-terminal-job",
                &graph,
                second,
                2,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
    }
    let control = engine.control_store().unwrap();
    let deadline = || Instant::now() + std::time::Duration::from_millis(50);
    engine
        .require_terminal_revision_ownerships(
            &[(revision, &scope), (second, &scope)],
            &control,
            deadline(),
        )
        .unwrap();
    let wrong_scope = diskgraph_core::ScopeId::new("other-terminal-scope").unwrap();
    assert!(matches!(
        engine.require_terminal_revision_ownerships(
            &[(revision, &scope), (second, &wrong_scope)],
            &control,
            deadline()
        ),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
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
        worker_engine.review_candidates(&revision, 0, QueryBudget::default(), &principal, &policy)
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
    assert!(matches!(
        answer,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
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
fn expired_envelope_still_observes_live_authorization_without_renewing_data_deadline() {
    let (dir, engine, principal, scope, revision) = published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    let expired = Instant::now() - std::time::Duration::from_millis(1);
    assert!(
        !engine
            .finalize_revision_read_until(revision, &principal, &policy, expired)
            .unwrap(),
        "authorization observation must not renew the expired data request"
    );
    {
        let _held = engine.control_store().unwrap();
        assert!(
            matches!(
                engine.finalize_revision_read_until(revision, &principal, &policy, expired),
                Err(EngineError::Business(BusinessError::BudgetExceeded))
            ),
            "terminal observation must refuse a held control lock"
        );
    }
    /// 模拟真实同步授权回调超过观察窗口，不给迟到允许结果续期。
    struct SlowPermit(diskgraph_core::PolicyAuthorizer);
    impl diskgraph_core::Authorizer for SlowPermit {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            std::thread::sleep(std::time::Duration::from_millis(80));
            self.0.decide(principal, permission, scope)
        }
    }
    let late_result = engine.finalize_revision_read_until(
        revision,
        &principal,
        &SlowPermit(engine.policy_authorizer().unwrap()),
        expired,
    );
    assert!(
        matches!(
            late_result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "a late capability decision must refuse even a truncated reply: {late_result:?}"
    );
    /// 慢回调通过独立真实控制连接撤权，返回允许也必须保留拒权优先。
    struct SlowRevoke {
        policy: diskgraph_core::PolicyAuthorizer,
        control: RefCell<diskgraph_store::ControlStore>,
    }
    impl diskgraph_core::Authorizer for SlowRevoke {
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            self.control.borrow_mut().revoke_scope(scope).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(80));
            self.policy.decide(principal, permission, scope)
        }
    }
    let slow_revoke = SlowRevoke {
        policy: engine.policy_authorizer().unwrap(),
        control: RefCell::new(
            diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap(),
        ),
    };
    assert!(
        matches!(
            engine.finalize_revision_read_until(revision, &principal, &slow_revoke, expired),
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "slow callback revocation must not be hidden by a budget error"
    );
    assert!(engine.scope(&scope).unwrap().revoked);
    assert!(matches!(
        engine.finalize_revision_read_until(revision, &principal, &policy, expired),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
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

#[test]
fn original_terminal_control_lock_is_refused_before_holder_release() {
    let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
    let engine = Arc::new(engine);
    let mut failures = Vec::new();
    for kind in 0..2 {
        let principal = principal.clone();
        let policy = engine.policy_authorizer().unwrap();
        // 无竞争时同一原授权路径仍正常返回；快拒绝不允许变成一律失败。
        let budget = QueryBudget::default();
        let deadline = query_deadline(budget).unwrap();
        if kind == 0 {
            engine
                .with_relation_reader_until(
                    revision,
                    &principal,
                    &policy,
                    deadline,
                    budget,
                    None,
                    |_, _, _| Ok(()),
                    |_, _| Ok(()),
                )
                .unwrap();
        } else {
            engine
                .with_history_readers_until(
                    revision,
                    revision,
                    &principal,
                    &policy,
                    deadline,
                    budget,
                    |_, _, _, _, _, _| Ok(()),
                    |_, _| Ok(()),
                )
                .unwrap();
        }
        let worker_engine = engine.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let budget = QueryBudget::default();
            let deadline = query_deadline(budget).unwrap();
            let consume = || {
                ready_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
                Ok(())
            };
            let result = if kind == 0 {
                worker_engine.with_relation_reader_until(
                    revision,
                    &principal,
                    &policy,
                    deadline,
                    budget,
                    None,
                    |_, _, _| consume(),
                    |_, _| Ok(()),
                )
            } else {
                worker_engine.with_history_readers_until(
                    revision,
                    revision,
                    &principal,
                    &policy,
                    deadline,
                    budget,
                    |_, _, _, _, _, _| consume(),
                    |_, _| Ok(()),
                )
            };
            result_tx.send(result).unwrap();
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let guard = engine.control_store().unwrap();
        resume_tx.send(()).unwrap();
        // 观察预算仅控制测试等待，不进入或延长原查询期限；失败后先释放原锁再 join。
        let observed = result_rx.recv_timeout(std::time::Duration::from_millis(200));
        drop(guard);
        worker.join().unwrap();
        if !matches!(
            observed,
            Ok(Err(EngineError::Business(BusinessError::BudgetExceeded)))
        ) {
            failures.push(format!("query kind {kind}: {observed:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "terminal control refusal failed: {failures:?}"
    );
}

/// 真实发布合法元数据夹具；参数：无；返回：隔离数据库、原 Engine、主体、scope 和 revision。
/// 不需要扫描部署，也不伪造任务或资源退休。
pub(super) fn published_authorization_fixture() -> (
    tempfile::TempDir,
    crate::Engine,
    PrincipalId,
    diskgraph_core::ScopeId,
    &'static str,
) {
    // 仅验证持久授权，不依赖扫描进程部署；通过真实 Store 发布合法的单节点快照。
    let dir = tempfile::tempdir().unwrap();
    let engine = crate::Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("expired-envelope-reader").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let scope = engine
        .register_scope(dir.path(), &principal, &engine.policy_authorizer().unwrap())
        .unwrap();
    let graph: diskgraph_core::DiskGraph = serde_json::from_value(serde_json::json!({
        "snapshot": {"id":"expired-envelope-snapshot", "root":{"type":"native_path","value":dir.path().to_string_lossy()},
            "volume_id":null,"captured_at_unix_ms":1,
            "settings":{"apparent_size":true,"follow_links":false,"include_hidden":true,
                "one_filesystem":true,"max_depth":null,"dedup_hardlinks":true},
            "coverage":{"complete":true,"unreadable_nodes":0,"depth_limited":false}},
        "nodes":[{"id":1,"parent_id":null,"locator":{"type":"native_path","value":dir.path().to_string_lossy()},
            "name":"root","kind":"directory","subtree_bytes":0,"direct_bytes":0,"size_known":true,
            "files":0,"directories":1,"modified_unix_seconds":null,"file_identity":null,
            "category_hint":null,"reclaim_hint":null,"read_error":false}],
        "evidence":[]
    })).unwrap();
    let revision = "expired-envelope-revision";
    let server = engine.server_id().unwrap();
    {
        let mut store = engine.graph().unwrap();
        store
            .append_staging_nodes("expired-envelope-job", &graph.nodes)
            .unwrap();
        store
            .publish_revision_owned(
                "expired-envelope-job",
                &graph,
                revision,
                1,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
    }
    (dir, engine, principal, scope, revision)
}

fn published_history_deadline_fixture(
    size_known: bool,
) -> (
    tempfile::TempDir,
    crate::Engine,
    PrincipalId,
    diskgraph_core::ScopeId,
    &'static str,
) {
    let (_dir, engine, principal, scope, original) = published_authorization_fixture();
    let revision = "history-data-expiry";
    let server = engine.server_id().unwrap();
    {
        let mut store = engine.graph().unwrap();
        let mut graph = store.load_revision(original).unwrap();
        graph.snapshot.id = "history-data-expiry-snapshot".into();
        graph.nodes[0].files = 1;
        let mut child = graph.nodes[0].clone();
        child.id = 2;
        child.parent_id = Some(1);
        child.name = "actual-row".into();
        child.kind = diskgraph_core::NodeKind::File;
        child.directories = 0;
        child.size_known = size_known;
        child.locator = serde_json::from_value(serde_json::json!({
            "type":"native_path", "value":_dir.path().join("actual-row")
        }))
        .unwrap();
        graph.nodes.push(child);
        store
            .append_staging_nodes("history-data-expiry-job", &graph.nodes)
            .unwrap();
        store
            .publish_revision_owned(
                "history-data-expiry-job",
                &graph,
                revision,
                2,
                Some((server.as_str(), scope.as_str())),
            )
            .unwrap();
    }
    (_dir, engine, principal, scope, revision)
}

#[test]
fn history_data_expiry_after_read_keeps_real_rows_with_timely_capability() {
    let (_dir, engine, principal, _scope, revision) = published_history_deadline_fixture(true);
    let policy = engine.policy_authorizer().unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(1);
    // 同步点只移动数据期限，不阻塞授权器，也不占有控制库 guard。
    AFTER_READ.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(|deadline| {
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now())
                    + std::time::Duration::from_millis(1),
            );
        }))
    });
    let report = engine
        .compare_revisions_until(
            revision,
            revision,
            0,
            QueryBudget::default(),
            &principal,
            &policy,
            deadline,
        )
        .unwrap();
    assert!(
        AFTER_READ.with(|slot| slot.borrow().is_none()),
        "actual post-read boundary must execute"
    );
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.summary.same, 1);
    let encoded = report.to_json(None);
    assert_eq!(encoded["complete"], false);
    assert_eq!(encoded["truncation_reason"], "deadline");
    assert_eq!(encoded["summary_is_partial"], true);
}

#[test]
fn terminal_relation_and_history_revocation_still_precedes_result_delivery() {
    for kind in 0..2 {
        let (dir, engine, principal, scope, revision) = published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        let mut independent =
            diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
                .unwrap();
        let budget = QueryBudget::default();
        let deadline = query_deadline(budget).unwrap();
        let encoded = std::cell::Cell::new(false);
        let mut consume = || {
            independent.revoke_scope(&scope).unwrap();
            Ok(())
        };
        let finish = |_: &mut (), _| {
            encoded.set(true);
            Ok(())
        };
        let result = if kind == 0 {
            engine.with_relation_reader_until(
                revision,
                &principal,
                &policy,
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
                &policy,
                deadline,
                budget,
                |_, _, _, _, _, _| consume(),
                finish,
            )
        };
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ));
        assert!(!encoded.get(), "revoked data reached encoder");
    }
}

#[test]
fn growth_data_expiry_with_timely_capability_is_timeout_for_unknown_and_cross_scope() {
    for cross_scope in [false, true] {
        let (dir, engine, principal, _scope, revision) =
            published_history_deadline_fixture(cross_scope);
        let after = if cross_scope {
            let root = dir.path().join("other-history-root");
            std::fs::create_dir(&root).unwrap();
            let other_scope = engine
                .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
                .unwrap();
            let server = engine.server_id().unwrap();
            let mut store = engine.graph().unwrap();
            let mut graph = store.load_revision(revision).unwrap();
            graph.snapshot.id = "other-history-expiry-snapshot".into();
            graph.snapshot.root =
                serde_json::from_value(serde_json::json!({"type":"native_path","value":root}))
                    .unwrap();
            for node in &mut graph.nodes {
                let path = if node.parent_id.is_none() {
                    root.clone()
                } else {
                    root.join(&node.name)
                };
                node.locator =
                    serde_json::from_value(serde_json::json!({"type":"native_path","value":path}))
                        .unwrap();
            }
            store
                .append_staging_nodes("other-history-expiry-job", &graph.nodes)
                .unwrap();
            store
                .publish_revision_owned(
                    "other-history-expiry-job",
                    &graph,
                    "other-history-expiry",
                    3,
                    Some((server.as_str(), other_scope.as_str())),
                )
                .unwrap();
            "other-history-expiry"
        } else {
            revision
        };
        let policy = engine.policy_authorizer().unwrap();
        let path = std::path::Path::new("actual-row");
        assert!(
            engine
                .growth_between_until(
                    revision,
                    after,
                    path,
                    QueryBudget::default(),
                    &principal,
                    &policy,
                    Instant::now() + std::time::Duration::from_secs(5)
                )
                .unwrap()
                .is_none()
        );
        AFTER_READ.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(|deadline| {
                std::thread::sleep(
                    deadline.saturating_duration_since(Instant::now())
                        + std::time::Duration::from_millis(1),
                );
            }))
        });
        let result = engine.growth_between_until(
            revision,
            after,
            path,
            QueryBudget::default(),
            &principal,
            &policy,
            Instant::now() + std::time::Duration::from_secs(1),
        );
        assert!(AFTER_READ.with(|slot| slot.borrow().is_none()));
        assert!(
            matches!(result, Err(EngineError::Business(BusinessError::Timeout))),
            "cross_scope={cross_scope}: {result:?}"
        );
    }
}

#[test]
fn candidate_and_impact_data_expiry_after_read_keep_timely_authorized_prefixes() {
    for impact in [false, true] {
        let (_dir, engine, principal, _scope, revision) = published_authorization_fixture();
        let policy = engine.policy_authorizer().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        // 初始授权和读取已完成，仅在真实读后边界耗尽原数据期限。
        AFTER_READ.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(|deadline| {
                std::thread::sleep(
                    deadline.saturating_duration_since(Instant::now())
                        + std::time::Duration::from_millis(1),
                );
            }));
        });
        if impact {
            let result = engine
                .revision_impact_until(
                    revision,
                    "absent",
                    QueryBudget::default(),
                    &principal,
                    &policy,
                    deadline,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "impact={impact} post_read_hook_consumed={} original_deadline_elapsed={} error={error:?}",
                        AFTER_READ.with(|slot| slot.borrow().is_none()),
                        Instant::now() >= deadline,
                    )
                });
            assert!(result.entries.is_empty());
            assert_eq!(
                result.truncated,
                Some(diskgraph_core::TruncationReason::Deadline)
            );
        } else {
            let result = engine
                .review_candidates_until(
                    revision,
                    0,
                    QueryBudget::default(),
                    &principal,
                    &policy,
                    deadline,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "impact={impact} post_read_hook_consumed={} original_deadline_elapsed={} error={error:?}",
                        AFTER_READ.with(|slot| slot.borrow().is_none()),
                        Instant::now() >= deadline,
                    )
                });
            assert!(result.candidates.is_empty());
            assert!(!result.complete);
            assert_eq!(
                result.truncated,
                Some(diskgraph_core::TruncationReason::Deadline)
            );
        }
        assert!(
            AFTER_READ.with(|slot| slot.borrow().is_none()),
            "actual post-read boundary must execute"
        );
    }
}

// 仅测试的回调结束同步点；不进入生产路径。
thread_local! {
    static AFTER_TERMINAL_CALLBACK: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(super) fn after_terminal_callback() {
    AFTER_TERMINAL_CALLBACK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[test]
fn timely_terminal_callback_does_not_pay_for_post_callback_observation() {
    let (_dir, engine, principal, scope, _revision) = published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    AFTER_TERMINAL_CALLBACK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            std::thread::sleep(std::time::Duration::from_millis(60))
        }));
    });
    // 能力决定已结束；后置观察拥有自己的新窗口，原数据期限没有续期。
    assert!(
        engine
            .observe_terminal_relation(&policy, &principal, &scope, None)
            .unwrap()
    );
    assert!(
        AFTER_TERMINAL_CALLBACK.with(|hook| hook.borrow().is_none()),
        "测试同步点必须实际消费"
    );
}
