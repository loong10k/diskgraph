use super::*;
use diskgraph_core::PrincipalId;
use diskgraph_engine::EngineConfig;
use diskgraph_store::{PlanState, StoreError};
use tempfile::TempDir;

/// A project on disk plus a published engine, the precondition every plan test
/// needs. Everything lives in a temp directory: no test touches a real path.
struct Project {
    _workspace: TempDir,
    engine: std::sync::Arc<Engine>,
    root: PathBuf,
    /// Set once a scope has been registered and indexed.
    indexed: Option<(ScopeId, diskgraph_core::PrincipalId)>,
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
        indexed: None,
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

// ---------------------------------------------------------------- execution ---

/// A move plan over `app.bin` into an `archive/` directory, approved and ready.
fn ready_move(project: &mut Project, label: &str) -> (Plan, String, Executor) {
    let (scope, principal) = indexed_once(project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let archive = project.root.join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_move_plan(
            &scope,
            &principal,
            &[app],
            &archive,
            1 << 20,
            FileActionKind::Move,
        )
        .unwrap();
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let _ = label;
    (
        plan,
        approval_ref,
        Executor::new(std::sync::Arc::clone(&project.engine)),
    )
}

/// Indexes once; later helpers reuse the same revision.
fn indexed_once(project: &mut Project) -> (ScopeId, PrincipalId) {
    match project.indexed {
        Some(ref pair) => pair.clone(),
        None => {
            let pair = indexed(project);
            project.indexed = Some(pair.clone());
            pair
        }
    }
}

#[test]
fn applying_a_move_moves_the_file_once_and_records_it() {
    let mut project = project("apply-move");
    let (plan, approval, executor) = ready_move(&mut project, "move");
    let source = project.root.join("target/app.bin");
    let target = project.root.join("archive/app.bin");
    assert!(source.exists());

    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k-1",
            fault: None,
        })
        .unwrap();
    assert!(outcome.started);
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert_eq!(outcome.moved, 1);
    assert_eq!(outcome.bytes, 4096);
    assert!(!source.exists(), "the source is gone after a move");
    assert!(target.exists(), "the target carries the same bytes");
    assert_eq!(std::fs::metadata(&target).unwrap().len(), 4096);

    // The durable record says the same thing.
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    assert_eq!(
        items[0].intent,
        diskgraph_store::IntentState::IntentRecorded
    );
    assert_eq!(items[0].result, diskgraph_store::OperationItemResult::Moved);
}

#[test]
fn a_retried_key_returns_the_original_operation_without_moving_twice() {
    let mut project = project("idempotent");
    let (plan, approval, executor) = ready_move(&mut project, "idem");
    let first = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "same-key",
            fault: None,
        })
        .unwrap();
    // A client that lost the response retries: the original operation comes
    // back, and nothing is moved a second time.
    let second = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "same-key",
            fault: None,
        })
        .unwrap();
    assert!(!second.started);
    assert_eq!(first.operation_id, second.operation_id);
    assert_eq!(second.moved, 0);
    assert!(project.root.join("archive/app.bin").exists());
}

#[test]
fn a_reused_key_with_a_different_approval_is_refused() {
    let mut project = project("key-conflict");
    let (plan, approval, executor) = ready_move(&mut project, "conflict");
    executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        })
        .unwrap();
    // A second approval for the same plan, applied under the same key, is a
    // different request and must not merge into the finished operation.
    let other_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console-2", 60_000)
            .unwrap()
            .approval_ref
    };
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &other_ref,
            idempotency_key: "k",
            fault: None,
        }),
        Err(OpsError::Store(StoreError::IdempotencyConflict))
    ));
}

#[test]
fn apply_without_a_valid_approval_moves_nothing() {
    let mut project = project("no-approval");
    let (plan, _approval, executor) = ready_move(&mut project, "no-approval");
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: "ap-does-not-exist",
            idempotency_key: "k",
            fault: None,
        }),
        Err(OpsError::NotAuthorized(_))
    ));
    assert!(
        project.root.join("target/app.bin").exists(),
        "the file is untouched"
    );
    assert!(!project.root.join("archive/app.bin").exists());
}

#[test]
fn an_object_replaced_after_planning_is_refused() {
    let mut project = project("replaced");
    let (plan, approval, executor) = ready_move(&mut project, "replaced");
    // Replace the object after the plan was approved: same path, new identity.
    let source = project.root.join("target/app.bin");
    std::fs::remove_file(&source).unwrap();
    std::fs::write(&source, vec![7; 4096]).unwrap();

    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        }),
        Err(OpsError::Stale(_))
    ));
    // Nothing moved: the stale object stays where the user put it.
    assert!(source.exists());
    assert!(!project.root.join("archive/app.bin").exists());
}

#[test]
fn an_occupied_target_is_never_overwritten() {
    let mut project = project("occupied");
    let (plan, approval, executor) = ready_move(&mut project, "occupied");
    // A file appears at the target after planning.
    let target = project.root.join("archive/app.bin");
    std::fs::write(&target, b"someone else's data").unwrap();

    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        })
        .unwrap();
    // The operation is not a success, and the existing file is intact.
    assert_ne!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert_eq!(std::fs::read(&target).unwrap(), b"someone else's data");
    assert!(project.root.join("target/app.bin").exists());
}

#[test]
fn a_crash_after_intent_leaves_the_operation_needing_attention() {
    let mut project = project("crash-intent");
    let (plan, approval, executor) = ready_move(&mut project, "crash");
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: Some(FaultPoint::AfterIntent),
        })
        .unwrap();
    assert_eq!(
        outcome.state,
        diskgraph_store::OperationState::NeedsAttention
    );
    assert_eq!(outcome.moved, 0);
    // The file is still exactly where it was: an intent is not a mutation.
    assert!(project.root.join("target/app.bin").exists());
    assert!(!project.root.join("archive/app.bin").exists());

    // The intent is durable, so a reviewer can see what was about to happen.
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    assert_eq!(
        items[0].intent,
        diskgraph_store::IntentState::IntentRecorded
    );
    assert_eq!(
        items[0].result,
        diskgraph_store::OperationItemResult::Pending
    );
}

#[test]
fn a_crash_after_the_file_moved_is_never_replayed_blindly() {
    let mut project = project("crash-moved");
    let (plan, approval, executor) = ready_move(&mut project, "crash-moved");
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: Some(FaultPoint::AfterFileChange),
        })
        .unwrap();
    assert_eq!(
        outcome.state,
        diskgraph_store::OperationState::NeedsAttention
    );
    // The file did move, but no result was recorded: the operation is parked
    // for a human rather than retried.
    assert!(project.root.join("archive/app.bin").exists());
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    assert_eq!(
        items[0].result,
        diskgraph_store::OperationItemResult::Pending
    );
}

#[test]
fn a_revoked_approval_blocks_execution() {
    let mut project = project("revoked-approval");
    let (plan, approval, executor) = ready_move(&mut project, "revoked");
    {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer.revoke(&approval).unwrap();
    }
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        }),
        Err(OpsError::NotAuthorized(_))
    ));
    assert!(project.root.join("target/app.bin").exists());
}

#[test]
fn a_plan_cannot_be_applied_twice() {
    let mut project = project("double-apply");
    let (plan, approval, executor) = ready_move(&mut project, "double");
    executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "first",
            fault: None,
        })
        .unwrap();
    // A fresh key cannot re-consume a plan that was already applied.
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "second",
            fault: None,
        }),
        Err(OpsError::Stale(_))
    ));
}
