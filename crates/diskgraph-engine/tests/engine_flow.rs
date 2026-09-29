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
        .related(
            &revision,
            &resource_id,
            None,
            true,
            &admin_principal,
            &admin_policy,
        )
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
        .explain_entity(&revision, &resource_id, &admin_principal, &admin_policy)
        .unwrap()
        .expect("resource entity exists");
    assert_eq!(entity.kind, diskgraph_core::EntityKind::Resource);
    assert!(!edges.is_empty());
    assert!(!evidence.is_empty());
    assert!(evidence.iter().all(|record| record.confidence <= 100));

    // An agent without metadata:read on the admin scope cannot explain.
    let unprivileged = PrincipalId::new("unprivileged").unwrap();
    let mut limited = PolicyAuthorizer::new(1);
    limited.grant(
        unprivileged.clone(),
        Permission::MetadataRead,
        scope_id.clone(),
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
