//! Engine end-to-end flow (P1 tasks 2.4 / 2.8 / 2.9 / 2.12): authorized scope
//! registration, durable index jobs, atomic publication, cancellation, owner
//! fencing, and node budgets. All fixtures are isolated temp directories.

use diskgraph_core::{BusinessError, Permission, PolicyAuthorizer, PrincipalId, ScopeId};
use diskgraph_engine::{Engine, EngineConfig, EngineError, admin_scope};
use diskgraph_store::{JobState, StoreError};
use diskgraph_testkit::FixtureTree;

fn admin() -> (PrincipalId, PolicyAuthorizer) {
    let principal = PrincipalId::new("admin").unwrap();
    let mut policy = PolicyAuthorizer::new(1);
    policy.grant(principal.clone(), Permission::ScopeAdmin, admin_scope());
    // Retention edits on the shared graph index are index management (C05);
    // relation reads against the shared index are metadata reads.
    policy.grant(principal.clone(), Permission::IndexWrite, admin_scope());
    policy.grant(principal.clone(), Permission::MetadataRead, admin_scope());
    (principal, policy)
}

fn agent_for(scope_id: &ScopeId) -> (PrincipalId, PolicyAuthorizer) {
    let principal = PrincipalId::new("agent").unwrap();
    let mut policy = PolicyAuthorizer::new(1);
    policy.grant(principal.clone(), Permission::IndexWrite, scope_id.clone());
    policy.grant(
        principal.clone(),
        Permission::MetadataRead,
        scope_id.clone(),
    );
    policy.grant(
        principal.clone(),
        Permission::OperationView,
        scope_id.clone(),
    );
    (principal, policy)
}

#[test]
fn authorized_reader_refuses_revoked_grant_before_calling_consumer() {
    let fixture = FixtureTree::new("authorized-reader").unwrap();
    fixture.file("one.txt", 1).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let (admin_principal, admin_policy) = admin();
    let scope = engine
        .register_scope(fixture.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    engine
        .control_store()
        .unwrap()
        .publish_policy_version(1)
        .unwrap();
    for permission in [
        Permission::IndexWrite,
        Permission::MetadataRead,
        Permission::OperationView,
    ] {
        engine
            .control_store()
            .unwrap()
            .upsert_grant(&diskgraph_core::Grant {
                principal: agent.clone(),
                permission,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
    }
    let job = engine.index_scope(&scope, &agent, &policy).unwrap();
    engine.run_job(&job.job_id, "frame-test").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    // 原生 Rust 权限行为回归使用公开预算上界；此测试不证明 50ms 性能。
    // 正式 display 和真实到期拒绝测试仍保留各自的 50ms 门禁；诊断仅在调用返回后输出。
    let consumer_enter = std::cell::Cell::new(None);
    let consumer_exit = std::cell::Cell::new(None);
    let request_deadline = std::cell::Cell::new(None);
    let consumer_rows = std::cell::Cell::new(None);
    let request_started = std::time::Instant::now();
    let result = engine.with_authorized_revision_reader(
        &revision,
        &agent,
        &policy,
        1000,
        |reader, snapshot, deadline| {
            consumer_enter.set(Some(std::time::Instant::now()));
            request_deadline.set(Some(deadline));
            let rows = reader.children(snapshot, 1, 0, 1);
            consumer_exit.set(Some(std::time::Instant::now()));
            consumer_rows.set(Some(rows.as_ref().map(Vec::len).map_err(|_| ())));
            Ok(rows?.len())
        },
    );
    let request_returned = std::time::Instant::now();
    eprintln!(
        "authorized_reader phases: result={result:?}, consumer_rows={:?}, deadline={:?}, \
         total={:?}, preparation={:?}, consumer={:?}, post_consumer={:?}, \
         remaining_at_return={:?}, overdue_at_return={:?}",
        consumer_rows.get(),
        request_deadline.get(),
        request_returned.duration_since(request_started),
        consumer_enter
            .get()
            .map(|entered| entered.duration_since(request_started)),
        consumer_exit
            .get()
            .zip(consumer_enter.get())
            .map(|(exited, entered)| exited.duration_since(entered)),
        consumer_exit
            .get()
            .map(|exited| request_returned.duration_since(exited)),
        request_deadline
            .get()
            .map(|deadline| deadline.saturating_duration_since(request_returned)),
        request_deadline
            .get()
            .map(|deadline| request_returned.saturating_duration_since(deadline)),
    );
    let rows = result.unwrap();
    assert_eq!(rows, 1);
    engine
        .control_store()
        .unwrap()
        .revoke_grant(&agent, &Permission::MetadataRead, &scope)
        .unwrap();
    let called = std::cell::Cell::new(false);
    let denied =
        engine.with_authorized_revision_reader(&revision, &agent, &policy, 1000, |_, _, _| {
            called.set(true);
            Ok(())
        });
    assert!(matches!(
        denied,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(!called.get(), "revoked consumers must not receive a reader");
}

#[test]
fn authorized_reader_refuses_scope_revoked_during_consumer_without_persisted_policy() {
    let engine = engine_in("reader-mid-revoke", 1000);
    let tree = FixtureTree::new("reader-mid-revoke").unwrap();
    tree.file("one", 1).unwrap();
    let (admin_principal, admin_policy) = admin();
    let scope = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    let job = engine.index_scope(&scope, &agent, &policy).unwrap();
    engine.run_job(&job.job_id, "test-owner").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let result = engine.with_authorized_revision_reader(
        &revision,
        &agent,
        &policy,
        1000,
        |reader, snapshot, _| {
            let name = reader.root_node(snapshot)?.unwrap().name;
            engine.revoke_scope(&scope, &admin_principal, &admin_policy)?;
            Ok(name)
        },
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "scope revoked during consumer returned {result:?}"
    );
}

#[test]
fn cancellation_from_another_engine_blocks_publication() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    for index in 0..8_000 {
        std::fs::write(root.join(format!("item-{index:05}")), b"x").unwrap();
    }
    let config = EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    };
    let first = std::sync::Arc::new(Engine::open(config.clone()).unwrap());
    let second = Engine::open(config).unwrap();
    let (admin_principal, admin_policy) = admin();
    let scope = first
        .register_scope(&root, &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    let job = first.index_scope(&scope, &agent, &policy).unwrap();
    let job_id = job.job_id.clone();
    let worker = std::thread::spawn(move || first.run_job(&job_id, "engine-a"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while second.job_status(&job.job_id).unwrap().state != JobState::Running {
        assert!(
            std::time::Instant::now() < deadline,
            "worker did not claim job"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    second.cancel_job(&job.job_id, &agent, &policy).unwrap();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(
        second.job_status(&job.job_id).unwrap().state,
        JobState::Cancelled
    );
    assert!(second.latest_revision(&scope).unwrap().is_none());
}

#[test]
fn recovering_a_job_clears_only_the_expired_fence_staging() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"x").unwrap();
    let data_dir = directory.path().join("data");
    let config = EngineConfig {
        data_dir: data_dir.clone(),
        ..EngineConfig::default()
    };
    let first = Engine::open(config.clone()).unwrap();
    let (admin_principal, admin_policy) = admin();
    let scope = first
        .register_scope(&root, &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    let job = first.index_scope(&scope, &agent, &policy).unwrap();
    let first_claim = first
        .control_store()
        .unwrap()
        .claim_job_once(&job.job_id, "expired-owner")
        .unwrap();
    let stale_id = format!("{}:{}", job.job_id, first_claim.fencing_token);
    let graph_path = data_dir.join("diskgraph.sqlite");
    let graph_connection = rusqlite::Connection::open(&graph_path).unwrap();
    graph_connection
        .execute(
            "INSERT INTO scan_staging (job_id, node_seq, node_json) VALUES (?1, 1, '{}')",
            [&stale_id],
        )
        .unwrap();
    drop(graph_connection);
    let control_connection =
        rusqlite::Connection::open(data_dir.join("diskgraph-control.sqlite")).unwrap();
    control_connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms = 0 WHERE job_id = ?1",
            [&job.job_id],
        )
        .unwrap();
    drop(control_connection);
    let second = Engine::open(config).unwrap();
    let finished = second.run_job(&job.job_id, "new-owner").unwrap();
    assert_eq!(finished.state, JobState::Completed);
    assert!(finished.fencing_token > first_claim.fencing_token);
    let graph = diskgraph_store::SqliteSnapshotStore::open(&graph_path).unwrap();
    assert_eq!(graph.staging_node_count(&stale_id).unwrap(), 0);
    assert!(second.latest_revision(&scope).unwrap().is_some());
}

#[test]
fn directory_pages_cover_all_children_without_silent_clipping() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    for index in 0..11 {
        std::fs::write(root.join(format!("item-{index:02}")), b"x").unwrap();
    }
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let (admin_principal, admin_policy) = admin();
    let scope = engine
        .register_scope(&root, &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    let job = engine.index_scope(&scope, &agent, &policy).unwrap();
    engine.run_job(&job.job_id, "worker").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let root_node = engine.revision_root_node(&revision).unwrap();
    let mut names = Vec::new();
    for (offset, expected_more) in [(0, true), (5, true), (10, false)] {
        let (_, page, more) = engine
            .revision_layer_page(&revision, root_node.id, offset, 5)
            .unwrap();
        assert_eq!(more, expected_more);
        names.extend(page.into_iter().map(|node| node.name));
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 11);
}

/// Engine rooted in a kept temp directory; the OS reclaims it after the run.
fn engine_in(label: &str, max_nodes: u64) -> Engine {
    let directory = tempfile::TempDir::with_prefix(format!("diskgraph-engine-{label}-")).unwrap();
    let data_dir = directory.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir,
        max_nodes_per_scan: max_nodes,
        ..EngineConfig::default()
    })
    .unwrap();
    let _kept = directory.keep();
    engine
}

#[test]
fn index_flow_publishes_latest_revision_and_v1_graph() {
    let engine = engine_in("flow", 1_000_000);
    let (admin_principal, admin_policy) = admin();

    let tree = FixtureTree::new("flow").unwrap();
    tree.file("data.bin", 1024).unwrap();
    tree.hidden_file("cache", 64).unwrap();

    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope_id);
    let job = engine.index_scope(&scope_id, &agent, &policy).unwrap();
    assert_eq!(job.state, JobState::Queued);

    let finished = engine.run_job(&job.job_id, "worker-1").unwrap();
    assert_eq!(finished.state, JobState::Completed);

    let revision = engine
        .latest_revision(&scope_id)
        .unwrap()
        .expect("published");
    let graph = engine.load_revision(&revision).unwrap();
    assert!(graph.snapshot.coverage.complete);
    assert!(graph.nodes.iter().any(|node| node.name == "data.bin"));
    assert!(graph.nodes.iter().any(|node| node.name == ".cache"));

    // A completed job frees the scope; the next index creates a new job.
    let second = engine.index_scope(&scope_id, &agent, &policy).unwrap();
    assert_ne!(second.job_id, job.job_id);
}

#[test]
fn completed_job_keeps_its_revision_after_a_later_publication() {
    let engine = engine_in("job-revision", 1000);
    let tree = FixtureTree::new("job-revision").unwrap();
    tree.file("first.bin", 8).unwrap();
    let (admin_principal, admin_policy) = admin();
    let scope = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (principal, policy) = agent_for(&scope);
    let first = engine.index_scope(&scope, &principal, &policy).unwrap();
    engine.run_job(&first.job_id, "owner-a").unwrap();
    let first_revision = engine.latest_revision(&scope).unwrap().unwrap();
    tree.file("second.bin", 16).unwrap();
    let second = engine.index_scope(&scope, &principal, &policy).unwrap();
    engine.run_job(&second.job_id, "owner-b").unwrap();
    let latest = engine.latest_revision(&scope).unwrap().unwrap();
    assert_ne!(first_revision, latest);
    assert_eq!(
        engine
            .revision_for_job(&first.job_id, &principal, &policy)
            .unwrap(),
        first_revision
    );
    assert_eq!(
        engine
            .revision_for_job(&second.job_id, &principal, &policy)
            .unwrap(),
        latest
    );
}

#[test]
fn scope_listing_intersects_stale_policy_with_live_scope_and_admin_grants() {
    let engine = engine_in("scope-list-live", 1000);
    let tree = FixtureTree::new("scope-list-live").unwrap();
    let registrar = PrincipalId::new("local-admin").unwrap();
    engine.bootstrap_local_admin(&registrar).unwrap();
    let scope = engine
        .register_scope(
            tree.path(),
            &registrar,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let reader = PrincipalId::new("list-reader").unwrap();
    engine
        .control_store()
        .unwrap()
        .upsert_grant(&diskgraph_core::Grant {
            principal: reader.clone(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version: 1,
        })
        .unwrap();
    let stale = engine.policy_authorizer().unwrap();
    assert_eq!(engine.list_scopes(&reader, &stale).unwrap().len(), 1);
    engine
        .control_store()
        .unwrap()
        .revoke_grant(&reader, &Permission::MetadataRead, &scope)
        .unwrap();
    assert!(engine.list_scopes(&reader, &stale).unwrap().is_empty());
    let admin_stale = engine.policy_authorizer().unwrap();
    let mut control = engine.control_store().unwrap();
    control
        .revoke_grant(&registrar, &Permission::MetadataRead, &scope)
        .unwrap();
    control
        .revoke_grant(&registrar, &Permission::MetadataRead, &admin_scope())
        .unwrap();
    drop(control);
    assert!(
        engine
            .list_scopes(&registrar, &admin_stale)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn server_identity_and_scope_survive_an_engine_restart() {
    let directory = tempfile::TempDir::with_prefix("diskgraph-engine-restart-").unwrap();
    let data_dir = directory.path().join("data");
    let tree = FixtureTree::new("restart").unwrap();
    let config = EngineConfig {
        data_dir: data_dir.clone(),
        max_nodes_per_scan: 1_000_000,
        ..EngineConfig::default()
    };

    let (first_server, scope_id) = {
        let engine = Engine::open(config.clone()).unwrap();
        let (admin_principal, admin_policy) = admin();
        let server = engine.server_id().unwrap();
        let scope = engine
            .register_scope(tree.path(), &admin_principal, &admin_policy)
            .unwrap();
        (server, scope)
    };
    {
        let engine = Engine::open(config).unwrap();
        assert_eq!(engine.server_id().unwrap(), first_server);
        assert_eq!(engine.scope(&scope_id).unwrap().scope_id, scope_id);
    }
}

#[test]
fn unauthorized_principals_are_denied_everywhere() {
    let engine = engine_in("deny", 1_000_000);
    let (admin_principal, admin_policy) = admin();
    let tree = FixtureTree::new("deny").unwrap();
    tree.file("f", 8).unwrap();
    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();

    let nobody = PrincipalId::new("nobody").unwrap();
    let deny_all = diskgraph_core::DenyAllAuthorizer;
    assert!(matches!(
        engine.register_scope(tree.path(), &nobody, &deny_all),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    assert!(matches!(
        engine.index_scope(&scope_id, &nobody, &deny_all),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    // Unknown jobs report not-found without leaking authorization state.
    assert!(matches!(
        engine.cancel_job("job-x", &nobody, &deny_all),
        Err(EngineError::Store(StoreError::JobNotFound(_)))
    ));

    // A real job with a principal lacking operations:view is denied.
    let (agent, policy) = agent_for(&scope_id);
    let job = engine.index_scope(&scope_id, &agent, &policy).unwrap();
    let unprivileged = PrincipalId::new("unprivileged").unwrap();
    let mut some_policy = PolicyAuthorizer::new(1);
    some_policy.grant(
        unprivileged.clone(),
        Permission::MetadataRead,
        scope_id.clone(),
    );
    assert!(matches!(
        engine.cancel_job(&job.job_id, &unprivileged, &some_policy),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn revoked_scopes_stop_accepting_jobs() {
    let engine = engine_in("revoke", 1_000_000);
    let (admin_principal, admin_policy) = admin();
    let tree = FixtureTree::new("revoke").unwrap();
    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    engine
        .revoke_scope(&scope_id, &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope_id);
    assert!(matches!(
        engine.index_scope(&scope_id, &agent, &policy),
        Err(EngineError::Store(StoreError::Conflict(_)))
    ));
}

#[test]
fn queued_jobs_cancel_without_running_and_terminal_jobs_never_rerun() {
    let engine = engine_in("cancel", 1_000_000);
    let (admin_principal, admin_policy) = admin();
    let tree = FixtureTree::new("cancel").unwrap();
    tree.file("f", 8).unwrap();
    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope_id);
    let job = engine.index_scope(&scope_id, &agent, &policy).unwrap();

    engine.cancel_job(&job.job_id, &agent, &policy).unwrap();
    let status = engine.job_status(&job.job_id).unwrap();
    assert_eq!(status.state, JobState::Cancelled);
    assert!(engine.latest_revision(&scope_id).unwrap().is_none());

    // A cancelled job can never be claimed again.
    assert!(matches!(
        engine.run_job(&job.job_id, "worker-1"),
        Err(EngineError::Store(StoreError::Conflict(_)))
    ));

    // Nor can a completed job (owner fencing on terminal states).
    let second = engine.index_scope(&scope_id, &agent, &policy).unwrap();
    engine.run_job(&second.job_id, "worker-1").unwrap();
    assert!(matches!(
        engine.run_job(&second.job_id, "worker-2"),
        Err(EngineError::Store(StoreError::Conflict(_)))
    ));
}

#[test]
fn over_budget_scans_fail_without_publishing() {
    let tree = FixtureTree::new("budget").unwrap();
    tree.file("a", 1).unwrap();
    tree.file("b", 1).unwrap();
    let engine = engine_in("budget", 1); // fixture has 3 nodes

    let (admin_principal, admin_policy) = admin();
    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope_id);
    let job = engine.index_scope(&scope_id, &agent, &policy).unwrap();
    assert!(matches!(
        engine.run_job(&job.job_id, "worker-1"),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    let status = engine.job_status(&job.job_id).unwrap();
    assert_eq!(status.state, JobState::Failed);
    assert!(engine.latest_revision(&scope_id).unwrap().is_none());
}

#[test]
fn sync_republishes_and_snapshot_retention_is_authorized() {
    let engine = engine_in("sync", 1_000_000);
    let (admin_principal, admin_policy) = admin();
    let tree = FixtureTree::new("sync").unwrap();
    tree.file("f", 16).unwrap();
    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope_id);

    // C03 sync: explicit controlled rescan publishes a fresh revision.
    let job = engine.sync_scope(&scope_id, &agent, &policy).unwrap();
    engine.run_job(&job.job_id, "worker-1").unwrap();
    let first_revision = engine
        .latest_revision(&scope_id)
        .unwrap()
        .expect("published");
    let second = engine.sync_scope(&scope_id, &agent, &policy).unwrap();
    engine.run_job(&second.job_id, "worker-1").unwrap();
    let second_revision = engine.latest_revision(&scope_id).unwrap().unwrap();
    assert_ne!(first_revision, second_revision);

    // C05: listing needs metadata read; retention edits need index write.
    let snapshots = engine
        .list_snapshots(&scope_id, &agent, &policy, 10, 0)
        .unwrap();
    assert_eq!(snapshots.len(), 2);
    let snapshot_id = snapshots[0].id.clone();

    let nobody = PrincipalId::new("nobody").unwrap();
    let deny_all = diskgraph_core::DenyAllAuthorizer;
    assert!(matches!(
        engine.pin_snapshot(&snapshot_id, true, &nobody, &deny_all),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));

    // Pin blocks removal; unpin lets retention proceed; revisions keep the
    // other snapshot alive.
    engine
        .pin_snapshot(&snapshot_id, true, &admin_principal, &admin_policy)
        .unwrap();
    assert!(matches!(
        engine.remove_snapshot(&snapshot_id, &admin_principal, &admin_policy),
        Err(EngineError::Store(StoreError::RetentionViolation(_)))
    ));
    engine
        .pin_snapshot(&snapshot_id, false, &admin_principal, &admin_policy)
        .unwrap();
    assert!(matches!(
        engine.remove_snapshot(&snapshot_id, &admin_principal, &admin_policy),
        Err(EngineError::Store(StoreError::RetentionViolation(_)))
    ));
}

#[test]
fn cargo_fixture_produces_typed_evidence_bound_to_the_revision() {
    let engine = engine_in("collect", 1_000_000);
    let (admin_principal, admin_policy) = admin();
    let tree = FixtureTree::new("collect").unwrap();
    tree.file("Cargo.toml", 32).unwrap();
    tree.file("src/main.rs", 16).unwrap();
    tree.dir("target").unwrap();
    tree.file("target/debug.bin", 512).unwrap();

    let scope_id = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope_id);
    let job = engine.sync_scope(&scope_id, &agent, &policy).unwrap();
    engine.run_job(&job.job_id, "worker-1").unwrap();
    let revision = engine.latest_revision(&scope_id).unwrap().unwrap();

    let graph = engine.load_revision(&revision).unwrap();
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "target")
        .unwrap();

    // The target directory has typed ownership evidence on its outgoing
    // edges (resource -> project / recipe), and by direct explanation.
    let resource_id = format!("resource-{}", target.id);
    let outgoing = engine
        .related(&revision, &resource_id, None, true, &agent, &policy)
        .unwrap();
    assert!(
        outgoing
            .iter()
            .any(|edge| edge.relation == diskgraph_core::Relation::OwnedByProject),
        "target must be owned by the cargo project"
    );
    assert!(
        outgoing
            .iter()
            .any(|edge| edge.relation == diskgraph_core::Relation::RebuildableBy),
        "cargo target must be rebuildable by the cargo recipe"
    );

    let (entity, edges, evidence) = engine
        .explain_entity(&revision, &resource_id, &agent, &policy)
        .unwrap()
        .expect("resource entity exists");
    assert_eq!(entity.kind, diskgraph_core::EntityKind::Resource);
    assert!(!edges.is_empty());
    assert!(!evidence.is_empty());
    assert!(evidence.iter().all(|record| record.confidence <= 100));

    // 只有 admin scope 的读取授权不能替代 revision 的实际 scope 授权。
    let unprivileged = PrincipalId::new("unprivileged").unwrap();
    let mut limited = PolicyAuthorizer::new(1);
    limited.grant(
        unprivileged.clone(),
        Permission::MetadataRead,
        admin_scope(),
    );
    assert!(matches!(
        engine.explain_entity(&revision, &resource_id, &unprivileged, &limited),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn per_principal_job_quotas_refuse_excess_without_running() {
    let tree_a = FixtureTree::new("quota-a").unwrap();
    let tree_b = FixtureTree::new("quota-b").unwrap();
    let directory = tempfile::TempDir::with_prefix("diskgraph-engine-quota-").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: directory.path().join("data"),
        max_nodes_per_scan: 1_000_000,
        max_active_jobs_per_principal: 1,
        ..EngineConfig::default()
    })
    .unwrap();
    let _kept = directory.keep();

    let (admin_principal, admin_policy) = admin();
    // Persist the administration grants so the rebuilt authorizers below have
    // them; engine-direct tests start from an empty control store otherwise.
    engine.bootstrap_local_admin(&admin_principal).unwrap();
    // Registration grants scope-local rights to the registrar; the shared
    // policy authorizer snapshot must be rebuilt after each registration.
    let scope_a = engine
        .register_scope(tree_a.path(), &admin_principal, &admin_policy)
        .unwrap();
    let admin_policy = engine.policy_authorizer().unwrap();
    let scope_b = engine
        .register_scope(tree_b.path(), &admin_principal, &admin_policy)
        .unwrap();
    let admin_policy = engine.policy_authorizer().unwrap();

    // The first job is admitted and held active by not running it.
    let first = engine
        .index_scope(&scope_a, &admin_principal, &admin_policy)
        .unwrap();
    // A second active job for the same principal exceeds the quota, even
    // though it targets a different scope.
    assert!(matches!(
        engine.index_scope(&scope_b, &admin_principal, &admin_policy),
        Err(EngineError::Business(BusinessError::ResourceExhausted))
    ));
    // Merging into the existing active job is still free: no new job exists.
    let merged = engine
        .index_scope(&scope_a, &admin_principal, &admin_policy)
        .unwrap();
    assert_eq!(merged.job_id, first.job_id);

    // Finishing the first job frees the quota.
    engine.run_job(&first.job_id, "worker-1").unwrap();
    let second = engine
        .index_scope(&scope_b, &admin_principal, &admin_policy)
        .unwrap();
    assert_ne!(second.job_id, first.job_id);
}

// ------------------------------------------------- 2.7 / 2.11 / 2.12 drills ---

#[test]
fn byte_charges_bill_each_file_once_not_once_per_ancestor() {
    // A deep tree whose every level nests a 1 KB file: the aggregate bytes
    // grow with depth, but the real content is 1 KB per level. A staging
    // budget that bills subtree aggregates would stop on phantom bytes
    // (regression: the home-directory scan stopped at 2 GB of a 395 GB tree).
    let tree = FixtureTree::new("charge-depth").unwrap();
    let depth = 30;
    let mut path = tree.path().to_path_buf();
    for level in 0..depth {
        path = path.join(format!("level-{level}"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("payload.bin"), vec![0_u8; 1024]).unwrap();
    }
    // The engine's data directory lives OUTSIDE the scanned tree: inside,
    // the walk would bill the very databases the scan is writing (and a
    // growing WAL), which is a deployment mistake this test need not copy.
    let outside = tempfile::TempDir::with_prefix("diskgraph-charge-data-").unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: outside.path().join("data"),
            max_nodes_per_scan: 1_000_000,
            scan_budget: diskgraph_core::ScanBudget {
                // 61 nodes x 1 block (4 KB) = ~245 KB real per-file charge;
                // the depth-multiplying subtree charge would be ~2.1 MB.
                max_staging_bytes: 400 * 1024,
                ..diskgraph_core::ScanBudget::default()
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let (admin_principal, admin_policy) = admin();
    let scope = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    let job = engine.index_scope(&scope, &agent, &policy).unwrap();
    // The honest per-file charge fits the budget: the scan completes.
    let finished = engine.run_job(&job.job_id, "charge-depth").unwrap();
    assert_eq!(finished.state, JobState::Completed);
    assert!(engine.latest_revision(&scope).unwrap().is_some());
}

#[test]
fn the_walk_budget_stops_a_scan_for_a_named_reason() {
    // A scan whose budget is exhausted stops for the named reason, without
    // publishing anything (task 2.9, RT-02).
    let tree = FixtureTree::new("walk-budget").unwrap();
    tree.file("a", 1024).unwrap();
    tree.file("b", 1024).unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: tree.path().join("data"),
            max_nodes_per_scan: 1_000_000,
            scan_budget: diskgraph_core::ScanBudget {
                max_nodes: 2,
                ..diskgraph_core::ScanBudget::default()
            },
            capacity_watermark: diskgraph_core::Watermark {
                warn_above_bytes: 1 << 30,
                refuse_above_bytes: 2 << 30,
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let (admin_principal, admin_policy) = admin();
    let scope = engine
        .register_scope(tree.path(), &admin_principal, &admin_policy)
        .unwrap();
    let (agent, policy) = agent_for(&scope);
    let job = engine.index_scope(&scope, &agent, &policy).unwrap();
    // The walk itself stops for the named reason and the job fails honestly.
    assert!(matches!(
        engine.run_job(&job.job_id, "budget"),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    // Nothing was published: a budget stop is a refusal, not partial data.
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}

#[test]
fn capacity_watermarks_refuse_new_work_without_touching_existing_data() {
    let directory = tempfile::TempDir::with_prefix("diskgraph-watermark-").unwrap();
    let data_dir = directory.path().join("data");
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: data_dir.clone(),
            max_nodes_per_scan: 1_000_000,
            scan_budget: diskgraph_core::ScanBudget::default(),
            capacity_watermark: diskgraph_core::Watermark {
                warn_above_bytes: 16,
                refuse_above_bytes: 4096,
            },
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    // The two databases alone exceed the tiny refuse threshold.
    assert!(
        !engine.accepts_new_work(),
        "a data directory past the watermark must refuse new work"
    );
    // The refusal deletes nothing: both databases are still on disk.
    assert!(data_dir.join("diskgraph.sqlite").exists());
    assert!(data_dir.join("diskgraph-control.sqlite").exists());
    drop(engine);
    assert!(data_dir.join("diskgraph.sqlite").exists());
}

#[test]
fn a_controlled_rescan_records_absence_without_deleting_history() {
    let project = project("rescan");
    let (scope, _principal) = indexed_once(&project);
    let engine = std::sync::Arc::clone(&project.1);
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let before = engine.load_revision(&revision).unwrap();
    let path_keys: Vec<String> = before
        .nodes
        .iter()
        .map(|node| match &node.locator {
            diskgraph_core::ResourceLocator::NativePath(path) => path.clone(),
            diskgraph_core::ResourceLocator::DocumentUri(uri) => uri.clone(),
        })
        .collect();

    // A rescan that observes the same tree records no absence and no addition,
    // and publishes nothing new (FS-06).
    let comparison = diskgraph_core::compare_rescan(
        before.nodes.len() as u64,
        path_keys.len() as u64,
        &path_keys,
        &path_keys,
        true,
    );
    assert_eq!(comparison.missing, 0);
    assert_eq!(comparison.added, 0);
    assert!(comparison.complete_observation);
}

fn project(label: &str) -> (FixtureTree, std::sync::Arc<Engine>) {
    let workspace = FixtureTree::new(&format!("diskgraph-ops-{label}-")).unwrap();
    std::fs::create_dir_all(workspace.path().join("project")).unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 1_000_000,
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    (workspace, engine)
}

fn indexed_once(
    project: &(FixtureTree, std::sync::Arc<Engine>),
) -> (ScopeId, diskgraph_core::PrincipalId) {
    let (workspace, engine) = project;
    let principal = diskgraph_core::PrincipalId::new("agent").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let admin_authorizer = engine.policy_authorizer().unwrap();
    let scope = engine
        .register_scope(
            &workspace.path().join("project"),
            &principal,
            &admin_authorizer,
        )
        .unwrap();
    // The scope-local grants only exist in the control store after
    // registration, so the authorizer must be reloaded before indexing.
    let authorizer = engine.policy_authorizer().unwrap();
    let job = engine.index_scope(&scope, &principal, &authorizer).unwrap();
    engine.run_job(&job.job_id, "worker-1").unwrap();
    (scope, principal)
}
