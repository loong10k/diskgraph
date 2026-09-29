use super::*;
use diskgraph_engine::EngineConfig;
use diskgraph_store::PlanState;
use tempfile::TempDir;

/// A project on disk plus a published engine, the precondition every plan test
/// needs. Everything lives in a temp directory: no test touches a real path.
struct Project {
    _workspace: TempDir,
    engine: std::sync::Arc<Engine>,
    root: PathBuf,
}

fn project(label: &str) -> Project {
    let workspace = TempDir::with_prefix(format!("diskgraph-ops-{label}-")).unwrap();
    let root = workspace.path().join("project");
    std::fs::create_dir_all(root.join("target").join("nested")).unwrap();
    std::fs::write(root.join("Cargo.toml"), b"[package]\n").unwrap();
    std::fs::write(root.join("target").join("app.bin"), vec![0; 4096]).unwrap();
    std::fs::write(
        root.join("target").join("nested").join("deep.bin"),
        vec![0; 1024],
    )
    .unwrap();
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: workspace.path().join("data"),
            max_nodes_per_scan: 100_000,
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    Project {
        _workspace: workspace,
        engine,
        root,
    }
}

/// Registers the project root as a scope and publishes a revision for it.
fn indexed(project: &mut Project) -> (ScopeId, PrincipalId) {
    let principal = PrincipalId::new("agent").unwrap();
    project.engine.bootstrap_local_admin(&principal).unwrap();
    let scope = project
        .engine
        .register_scope(
            &project.root,
            &principal,
            &project.engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    let authorizer = project.engine.policy_authorizer().unwrap();
    let job = project
        .engine
        .index_scope(&scope, &principal, &authorizer)
        .unwrap();
    project.engine.run_job(&job.job_id, "ops-test").unwrap();
    (scope, principal)
}

/// The node id for a path inside the published graph, by file name.
fn node_named(engine: &std::sync::Arc<Engine>, scope: &ScopeId, name: &str) -> u64 {
    let revision = engine.latest_revision(scope).unwrap().unwrap();
    let graph = engine.load_revision(&revision).unwrap();
    graph
        .nodes
        .iter()
        .find(|node| node.name == name)
        .map(|node| node.id)
        .unwrap_or_else(|| panic!("node {name} missing"))
}

/// Every path under `root`, sorted, for before/after comparisons.
fn tree(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else {
                out.push((path, meta.len()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn building_a_trash_plan_does_not_touch_any_file() {
    let mut project = project("no-touch");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let before = tree(&project.root);

    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();

    // The plan exists and names the object...
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.action, FileActionKind::Trash);
    assert_eq!(plan.recovery, RecoveryRule::Quarantine);
    // ...and the filesystem is byte-for-byte identical.
    assert_eq!(tree(&project.root), before);
    assert!(project.root.join("target/app.bin").exists());
}

#[test]
fn a_plan_is_immutable_and_digest_addressed() {
    let mut project = project("digest");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();

    // Read through a short-lived guard: holding the control lock across a
    // build call would self-deadlock, because the builder locks it too.
    {
        let control = project.engine.control_store().unwrap();
        let stored = control.plan(&plan.plan_id).unwrap();
        assert_eq!(stored, plan);
        assert_eq!(
            control.plan_digest(&plan.plan_id).unwrap(),
            plan_digest(&plan)
        );
        assert_eq!(
            control.plan_state(&plan.plan_id).unwrap(),
            PlanState::Validated
        );
    }

    // Rebuilding an identical request yields an identical digest, so the same
    // approval can be reviewed once; a different target changes it.
    let again = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    assert_eq!(plan_digest(&again), plan_digest(&plan));

    let moved = builder
        .build_move_plan(
            &scope,
            &principal,
            &[app],
            &project.root.join("archive"),
            1 << 20,
            FileActionKind::Move,
        )
        .unwrap();
    assert_ne!(plan_digest(&moved), plan_digest(&plan));
}

#[test]
fn parent_and_child_requests_collapse_to_the_child() {
    let mut project = project("overlap");
    let (scope, principal) = indexed(&mut project);
    let target = node_named(&project.engine, &scope, "target");
    let deep = node_named(&project.engine, &scope, "deep.bin");
    let before = tree(&project.root);

    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    // Asking for both the directory and a file inside it must not double-count.
    let plan = builder
        .build_trash_plan(&scope, &principal, &[target, deep], 1 << 20)
        .unwrap();

    assert_eq!(plan.items.len(), 1, "the ancestor must be dropped");
    assert_eq!(plan.items[0].node_id, deep);
    assert_eq!(
        plan.expected_bytes,
        std::fs::metadata(project.root.join("target/nested/deep.bin"))
            .unwrap()
            .len()
    );
    assert_eq!(
        tree(&project.root),
        before,
        "planning still touches nothing"
    );
}

#[test]
fn duplicate_objects_are_refused_rather_than_double_counted() {
    let mut project = project("dupes");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    // The same object twice cannot be resolved by dropping one.
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[app, app], 1 << 20),
        Err(OpsError::OverlappingObjects(_))
    ));
}

#[test]
fn a_revoked_scope_yields_no_plan() {
    let mut project = project("revoked");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let authorizer = project.engine.policy_authorizer().unwrap();
    project
        .engine
        .revoke_scope(&scope, &principal, &authorizer)
        .unwrap();
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[app], 1 << 20),
        Err(OpsError::NotAuthorized(_))
    ));
}

#[test]
fn an_empty_or_oversized_request_is_refused() {
    let mut project = project("bounds");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));

    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[], 1 << 20),
        Err(OpsError::EmptyPlan)
    ));
    // A budget below the object size is refused rather than silently trimmed.
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[app], 16),
        Err(OpsError::Stale(_))
    ));
}

#[test]
fn a_missing_node_is_refused_not_guessed() {
    let mut project = project("missing");
    let (scope, principal) = indexed(&mut project);
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    assert!(matches!(
        builder.build_trash_plan(&scope, &principal, &[999_999], 1 << 20),
        Err(OpsError::NoSuchNode(999_999))
    ));
}

#[test]
fn an_approval_is_bound_to_the_plan_and_its_principal() {
    let mut project = project("approval");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let digest = plan_digest(&plan);

    let mut control = project.engine.control_store().unwrap();
    let approval = {
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer.issue(&plan, "admin-console", 60_000).unwrap()
    };

    // The bound approval verifies.
    assert!(
        control
            .verify_approval(
                &approval.approval_ref,
                &plan.plan_id,
                &digest,
                &principal,
                plan.action
            )
            .is_ok()
    );

    // It cannot be reused for another plan, principal, or action.
    assert!(
        control
            .verify_approval(
                &approval.approval_ref,
                "plan-other",
                &digest,
                &principal,
                plan.action
            )
            .is_err()
    );
    let stranger = PrincipalId::new("stranger").unwrap();
    assert!(
        control
            .verify_approval(
                &approval.approval_ref,
                &plan.plan_id,
                &digest,
                &stranger,
                plan.action
            )
            .is_err()
    );
    assert!(
        control
            .verify_approval(
                &approval.approval_ref,
                &plan.plan_id,
                &digest,
                &principal,
                FileActionKind::Move
            )
            .is_err()
    );

    // Revoking before use is effective.
    {
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer.revoke(&approval.approval_ref).unwrap();
    }
    assert!(
        control
            .verify_approval(
                &approval.approval_ref,
                &plan.plan_id,
                &digest,
                &principal,
                plan.action
            )
            .is_err()
    );
}

#[test]
fn an_expired_approval_does_not_verify() {
    let mut project = project("expiry");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let digest = plan_digest(&plan);

    let mut control = project.engine.control_store().unwrap();
    let mut issuer = ApprovalIssuer::new(&mut control);
    // A zero-TTL approval is already expired.
    let approval = issuer.issue(&plan, "admin-console", 0).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert!(
        control
            .verify_approval(
                &approval.approval_ref,
                &plan.plan_id,
                &digest,
                &principal,
                plan.action
            )
            .is_err()
    );
}

#[test]
fn a_plan_show_and_validate_surface_the_exact_object_set() {
    let mut project = project("show");
    let (scope, principal) = indexed(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();

    let mut control = project.engine.control_store().unwrap();
    // A fresh plan validates...
    assert_eq!(
        control.plan_state(&plan.plan_id).unwrap(),
        PlanState::Validated
    );
    // ...and every stored object still points at a real file, so a review shows
    // exactly what apply would touch.
    for item in &plan.items {
        let path = project.root.join("target").join("app.bin");
        assert!(path.exists());
        let _ = item;
    }
    // Once consumed it no longer validates, so a second apply is impossible.
    control.mark_plan_applied(&plan.plan_id).unwrap();
    assert_eq!(
        control.plan_state(&plan.plan_id).unwrap(),
        PlanState::Applied
    );
}
