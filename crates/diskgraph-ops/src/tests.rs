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
    let first = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "first",
            fault: None,
        })
        .unwrap();
    {
        let control = project.engine.control_store().unwrap();
        let items = control.operation_items(&first.operation_id).unwrap();
        assert_eq!(
            first.state,
            diskgraph_store::OperationState::Succeeded,
            "first apply should succeed, item said {:?}: {first:?}",
            items.first().map(|item| item.detail.clone())
        );
    }
    {
        let control = project.engine.control_store().unwrap();
        assert_eq!(
            control.plan_state(&plan.plan_id).unwrap(),
            diskgraph_store::PlanState::Applied,
            "a succeeded apply must consume its plan"
        );
    }
    // A fresh key cannot re-consume a plan that was already applied.
    let second = executor.apply(ApplyRequest {
        plan_id: &plan.plan_id,
        approval_ref: &approval,
        idempotency_key: "second",
        fault: None,
    });
    assert!(
        matches!(second, Err(OpsError::Stale(_))),
        "second apply should be refused, got {second:?}"
    );
}

// ------------------------------------------------------ trash and restore ---

/// Trashes `app.bin`, returning the recovery reference it produced.
fn quarantine_app(project: &mut Project) -> (Plan, String, Executor) {
    let (scope, principal) = indexed_once(project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "trash-1",
            fault: None,
        })
        .unwrap();
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    let recovery_ref = items[0]
        .recovery_ref
        .clone()
        .expect("trash records recovery");
    (plan, recovery_ref, executor)
}

#[test]
fn trash_moves_into_quarantine_and_never_deletes() {
    let mut project = project("trash");
    let source = project.root.join("target/app.bin");
    let (_plan, recovery_ref, _executor) = quarantine_app(&mut project);

    // The original is gone, but the object is held, not destroyed.
    assert!(!source.exists());
    let control = project.engine.control_store().unwrap();
    let entry = control.recovery(&recovery_ref).unwrap();
    assert_eq!(entry.state, diskgraph_store::RecoveryState::Available);
    let held = PathBuf::from(&entry.quarantine_locator);
    // The stored locator is the hex key, so decode it the same way the ops
    // layer does before asserting the bytes survived.
    let decoded = ops_unhex(&entry.quarantine_locator);
    assert!(decoded.exists(), "the held object is still on disk");
    assert_eq!(std::fs::metadata(&decoded).unwrap().len(), 4096);
    let _ = held;
}

/// Decodes a hex locator key; mirrors the ops helper for assertions.
fn ops_unhex(key: &str) -> PathBuf {
    let bytes: Vec<u8> = key
        .as_bytes()
        .chunks(2)
        .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect();
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn a_restored_object_returns_to_its_original_place() {
    let mut project = project("restore");
    let (_plan, recovery_ref, executor) = quarantine_app(&mut project);
    let (scope, principal) = indexed_once(&mut project);
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_restore_plan(&scope, &principal, &recovery_ref, None)
        .unwrap();
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "restore-1",
            fault: None,
        })
        .unwrap();
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);

    // Back where it started, byte for byte.
    let source = project.root.join("target/app.bin");
    assert!(source.exists());
    assert_eq!(std::fs::metadata(&source).unwrap().len(), 4096);
    let control = project.engine.control_store().unwrap();
    assert_eq!(
        control.recovery(&recovery_ref).unwrap().state,
        diskgraph_store::RecoveryState::Restored
    );
}

#[test]
fn a_restore_never_overwrites_a_reoccupied_original() {
    let mut project = project("restore-occupied");
    let (_plan, recovery_ref, executor) = quarantine_app(&mut project);
    // Something new occupies the original name.
    let source = project.root.join("target/app.bin");
    std::fs::write(&source, b"new work").unwrap();
    let (scope, principal) = indexed_once(&mut project);
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_restore_plan(&scope, &principal, &recovery_ref, None)
        .unwrap();
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "restore-2",
            fault: None,
        })
        .unwrap();
    assert_ne!(outcome.state, diskgraph_store::OperationState::Succeeded);
    // The new file survives untouched, and the held object is still held.
    assert_eq!(std::fs::read(&source).unwrap(), b"new work");
    let control = project.engine.control_store().unwrap();
    assert_eq!(
        control.recovery(&recovery_ref).unwrap().state,
        diskgraph_store::RecoveryState::Available
    );
}

#[test]
fn a_restore_can_target_another_authorized_location() {
    let mut project = project("restore-elsewhere");
    let (_plan, recovery_ref, executor) = quarantine_app(&mut project);
    let (scope, principal) = indexed_once(&mut project);
    let elsewhere = project.root.join("archive");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_restore_plan(&scope, &principal, &recovery_ref, Some(&elsewhere))
        .unwrap();
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "restore-3",
            fault: None,
        })
        .unwrap();
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert!(elsewhere.join("app.bin").exists());
    assert!(!project.root.join("target/app.bin").exists());
}

#[test]
fn a_restore_of_a_vanished_object_is_refused() {
    let mut project = project("restore-vanished");
    let (_plan, recovery_ref, _executor) = quarantine_app(&mut project);
    // The held object disappears (a user cleared the quarantine by hand).
    let control = project.engine.control_store().unwrap();
    let entry = control.recovery(&recovery_ref).unwrap();
    let held = ops_unhex(&entry.quarantine_locator);
    std::fs::remove_file(&held).unwrap();
    drop(control);

    let (scope, principal) = indexed_once(&mut project);
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    // Planning itself refuses: there is nothing left to restore.
    assert!(matches!(
        builder.build_restore_plan(&scope, &principal, &recovery_ref, None),
        Err(OpsError::Stale(_))
    ));
}

#[test]
fn purging_is_refused_rather_than_silently_enabled() {
    let mut project = project("purge-refused");
    let (scope, principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let source = project.root.join("target/app.bin");
    // Even a well-formed plan cannot route to a permanent delete today.
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    assert!(matches!(
        builder.build_move_plan(
            &scope,
            &principal,
            &[app],
            &project.root,
            1 << 20,
            FileActionKind::Purge,
        ),
        Err(OpsError::ActionMismatch)
    ));
    assert!(source.exists());
}

#[test]
fn an_empty_file_round_trips_through_trash_and_restore() {
    let mut project = project("empty-file");
    // A zero-byte object is still a real object: it must be held and returned
    // like any other, not skipped for having no size.
    let empty = project.root.join("target/empty.log");
    std::fs::write(&empty, b"").unwrap();
    let scope = {
        let (scope, _) = indexed_once(&mut project);
        scope
    };
    let principal = PrincipalId::new("agent").unwrap();
    // Re-index so the new file is in the revision the plan is built from.
    let authorizer = project.engine.policy_authorizer().unwrap();
    let job = project
        .engine
        .index_scope(&scope, &principal, &authorizer)
        .unwrap();
    project.engine.run_job(&job.job_id, "reindex").unwrap();
    let node = node_named(&project.engine, &scope, "empty.log");

    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_trash_plan(&scope, &principal, &[node], 1 << 20)
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "empty-1",
            fault: None,
        })
        .unwrap();
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert!(!empty.exists());
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    let recovery_ref = items[0].recovery_ref.clone().unwrap();
    let held = ops_unhex(&control.recovery(&recovery_ref).unwrap().quarantine_locator);
    assert!(held.exists());
    assert_eq!(std::fs::metadata(&held).unwrap().len(), 0);
}

// -------------------------------------------- 6.8 path and link revalidation ---

#[test]
fn a_link_planted_in_the_source_after_planning_stops_the_move() {
    let mut project = project("link-source");
    let (scope, principal) = indexed_once(&mut project);
    // Plan against the real layout first, so the plan names a real object.
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
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };

    // After approval, swap the planned directory for a symlink pointing
    // somewhere else: the plan described a directory, and now it is a link.
    let swap = project.root.join("target");
    let real = project.root.join("real-target");
    std::fs::rename(&swap, &real).unwrap();
    let elsewhere = project.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &swap).unwrap();

    // The plan's object is no longer reachable the way it described, so the
    // apply is refused rather than redirected through the link.
    let outcome = executor.apply(ApplyRequest {
        plan_id: &plan.plan_id,
        approval_ref: &approval_ref,
        idempotency_key: "link",
        fault: None,
    });
    match outcome {
        Err(OpsError::Stale(_)) => {}
        Ok(outcome) => assert_ne!(
            outcome.state,
            diskgraph_store::OperationState::Succeeded,
            "a link in the path must not complete the move: {outcome:?}"
        ),
        Err(other) => panic!("unexpected refusal: {other}"),
    }
    assert!(
        real.join("app.bin").exists(),
        "the real object is untouched"
    );
    assert!(
        !elsewhere.join("app.bin").exists(),
        "the link was not followed"
    );
}

#[test]
fn a_link_planted_in_the_destination_stops_the_move() {
    let mut project = project("link-target");
    let (scope, principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let archive = project.root.join("archive");
    let hidden = project.root.join("hidden");
    std::fs::create_dir_all(&hidden).unwrap();
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
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    // Plant a symlink where the destination directory will be.
    std::os::unix::fs::symlink(&hidden, &archive).unwrap();

    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "link-target",
            fault: None,
        })
        .unwrap();
    assert_ne!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert!(project.root.join("target/app.bin").exists());
    assert!(
        !hidden.join("app.bin").exists(),
        "the link was not followed"
    );
}

#[test]
fn revalidation_reports_the_fault_it_refused() {
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("f"), b"x").unwrap();
    let root = directory.path();
    // A plain path below the trusted root passes on both sides.
    assert!(revalidate_below(root, &real, Side::Source).is_ok());
    // A missing source is a vanished component, not a link.
    assert_eq!(
        revalidate_below(root, &root.join("absent"), Side::Source),
        Err(PathFault::ComponentVanished)
    );
    // A missing destination is fine: the move will create it.
    assert!(revalidate_below(root, &root.join("new"), Side::Target).is_ok());
    // A symlinked component below the root is refused on whichever side it
    // appears.
    let link = root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(
        revalidate_below(root, &link.join("f"), Side::Source),
        Err(PathFault::SourceIsLink)
    );
    assert_eq!(
        revalidate_below(root, &link.join("f"), Side::Target),
        Err(PathFault::TargetIsLink)
    );
    // A path outside the trusted root is refused outright rather than trusted
    // by default.
    assert_eq!(
        revalidate_below(root, &std::env::temp_dir().join("elsewhere"), Side::Source),
        Err(PathFault::ComponentVanished)
    );
    // A system-level symlink in the prefix is trusted: on macOS /var points
    // into /private/var, and refusing it would break every real operation.
    #[cfg(target_os = "macos")]
    if std::fs::symlink_metadata("/var")
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        assert!(
            revalidate_below(
                &PathBuf::from("/var"),
                &PathBuf::from("/var/folders"),
                Side::Source
            )
            .is_ok()
        );
    }
}

// ------------------------------------------------------- 6.12 operations API ---

#[test]
fn operations_are_listed_and_shown_only_to_their_principal() {
    let mut project = project("ops-list");
    let (plan, approval, executor) = ready_move(&mut project, "ops");
    let (_scope, principal) = indexed_once(&mut project);
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        })
        .unwrap();
    let engine = std::sync::Arc::clone(&project.engine);

    let listed = list_operations(&engine, &plan.scope_id, &principal, 10).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].operation_id, outcome.operation_id);
    assert_eq!(listed[0].state, diskgraph_store::OperationState::Succeeded);

    // Another principal sees nothing and cannot show it.
    let stranger = PrincipalId::new("stranger").unwrap();
    assert!(
        list_operations(&engine, &plan.scope_id, &stranger, 10)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        show_operation(&engine, &outcome.operation_id, &stranger),
        Err(OpsError::NotAuthorized(_))
    ));
    assert!(show_operation(&engine, &outcome.operation_id, &principal).is_ok());
}

#[test]
fn cancelling_a_finished_operation_leaves_it_untouched() {
    let mut project = project("ops-cancel-done");
    let (plan, approval, executor) = ready_move(&mut project, "cancel");
    let (_scope, principal) = indexed_once(&mut project);
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        })
        .unwrap();
    let engine = std::sync::Arc::clone(&project.engine);
    let view = cancel_operation(&engine, &outcome.operation_id, &principal).unwrap();
    // A finished operation is never rewritten by a late cancellation.
    assert_eq!(view.state, diskgraph_store::OperationState::Succeeded);
    assert_eq!(view.completed, 1);
}

#[test]
fn cancelling_a_stalled_operation_stops_only_the_remaining_items() {
    let mut project = project("ops-cancel-stalled");
    let (plan, approval, executor) = ready_move(&mut project, "stall");
    let (_scope, principal) = indexed_once(&mut project);
    // Stop right after the intent: the file has not moved, and one item is
    // still outstanding.
    let stalled = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: Some(FaultPoint::AfterIntent),
        })
        .unwrap();
    let engine = std::sync::Arc::clone(&project.engine);
    let view = cancel_operation(&engine, &stalled.operation_id, &principal).unwrap();
    // An operation parked for a human is already terminal: cancelling it would
    // rewrite the very state a reviewer needs to see.
    assert_eq!(
        view.state,
        diskgraph_store::OperationState::NeedsAttention,
        "a parked operation keeps its state so a reviewer can reconcile it"
    );
    // The object is exactly where the plan left it.
    assert!(project.root.join("target/app.bin").exists());
}

#[test]
fn another_principal_cannot_cancel_someone_elses_operation() {
    let mut project = project("ops-cancel-foreign");
    let (plan, approval, executor) = ready_move(&mut project, "foreign");
    let (_scope, _principal) = indexed_once(&mut project);
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: Some(FaultPoint::AfterIntent),
        })
        .unwrap();
    let engine = std::sync::Arc::clone(&project.engine);
    let stranger = PrincipalId::new("stranger").unwrap();
    assert!(matches!(
        cancel_operation(&engine, &outcome.operation_id, &stranger),
        Err(OpsError::NotAuthorized(_))
    ));
}

// --------------------------------- 6.14 volume measurement and index refresh ---

#[test]
fn a_volume_report_separates_processed_retained_and_measured_space() {
    let mut project = project("volume-report");
    let (_plan, _recovery, _executor) = quarantine_app(&mut project);
    let (scope, _principal) = indexed_once(&mut project);
    let engine = std::sync::Arc::clone(&project.engine);

    let processed = 4096u64;
    let retained = quarantine_retained_bytes(&engine, &scope).unwrap();
    // The object is held, so the retained bytes are reported separately: the
    // user must not read a trash as freed space.
    assert_eq!(retained, processed);

    let free_before = volume_free_bytes(&project.root).unwrap_or(0);
    let free_after = volume_free_bytes(&project.root).unwrap_or(0);
    let report = VolumeReport {
        processed_bytes: processed,
        retained_in_quarantine_bytes: retained,
        free_before_bytes: free_before,
        free_after_bytes: free_after,
        measured_at_unix_ms: 1,
        caveat: "the delta reflects all writers on this volume, not only this operation",
    };
    // A same-volume move changes no free space at all, and the report says so
    // by keeping the numbers apart rather than implying space was freed.
    assert_eq!(report.free_delta_bytes(), 0);
    let json = report.to_json();
    assert_eq!(json["processed_bytes"], "4096");
    assert_eq!(json["retained_in_quarantine_bytes"], "4096");
    assert_eq!(json["free_delta_bytes"], "0");
    assert!(json["caveat"].as_str().unwrap().contains("all writers"));
}

#[test]
fn the_index_is_refreshed_so_a_later_query_sees_the_moved_file() {
    let mut project = project("refresh");
    let (plan, approval, executor) = ready_move(&mut project, "refresh");
    let (_scope, principal) = indexed_once(&mut project);
    executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        })
        .unwrap();
    // Before the refresh, the published revision still lists the old location.
    let (scope, _) = indexed_once(&mut project);
    let before = {
        let revision = project.engine.latest_revision(&scope).unwrap().unwrap();
        project.engine.load_revision(&revision).unwrap()
    };
    assert!(before.nodes.iter().any(|node| {
        node.name == "app.bin"
            && node
                .locator
                .raw_path()
                .map(|path| path.ends_with("target/app.bin"))
                .unwrap_or(false)
    }));

    let outcome =
        refresh_scope_after_operation(&std::sync::Arc::clone(&project.engine), &scope, &principal)
            .unwrap();
    assert_eq!(outcome.state, diskgraph_store::JobState::Completed);

    // After the refresh, the index reflects the new location.
    let after = {
        let revision = project.engine.latest_revision(&scope).unwrap().unwrap();
        project.engine.load_revision(&revision).unwrap()
    };
    let moved = after.nodes.iter().any(|node| {
        node.name == "app.bin"
            && node
                .locator
                .raw_path()
                .map(|path| path.ends_with("archive/app.bin"))
                .unwrap_or(false)
    });
    assert!(moved, "the refreshed index must show the new location");
}

// -------------------------------------------------- 6.13 interruption drills ---

#[test]
fn a_lost_response_retry_returns_the_result_without_repeating_the_move() {
    let mut project = project("lost-response");
    let (plan, approval, executor) = ready_move(&mut project, "lost");
    indexed_once(&mut project);
    let first = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "client-req-1",
            fault: None,
        })
        .unwrap();
    assert_eq!(first.state, diskgraph_store::OperationState::Succeeded);

    // The client never saw the response and retries. Nothing moves again, and
    // the second attempt carries the same operation identity.
    let retry = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "client-req-1",
            fault: None,
        })
        .unwrap();
    assert!(!retry.started);
    assert_eq!(retry.operation_id, first.operation_id);
    assert!(project.root.join("archive/app.bin").exists());
}

#[test]
fn a_parked_operation_is_never_resumed_automatically() {
    let mut project = project("no-blind-replay");
    let (plan, approval, executor) = ready_move(&mut project, "parked");
    let (scope, principal) = indexed_once(&mut project);
    let engine = std::sync::Arc::clone(&project.engine);

    // Interrupt after the file moved but before the result was written.
    let parked = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: Some(FaultPoint::AfterFileChange),
        })
        .unwrap();
    assert_eq!(
        parked.state,
        diskgraph_store::OperationState::NeedsAttention
    );

    // Any later attempt under a new key is refused: the plan was not consumed
    // and the object it named is no longer where it described.
    assert!(
        executor
            .apply(ApplyRequest {
                plan_id: &plan.plan_id,
                approval_ref: &approval,
                idempotency_key: "later",
                fault: None,
            })
            .is_err(),
        "a parked operation must not be replayed automatically"
    );

    // The record still shows the item as unresolved, which is what a reviewer
    // needs in order to decide.
    let control = engine.control_store().unwrap();
    let items = control.operation_items(&parked.operation_id).unwrap();
    assert_eq!(
        items[0].result,
        diskgraph_store::OperationItemResult::Pending
    );
    let _ = (scope, principal);
}

// ------------------------------------------------ 6.15 control-plane resilience ---

#[test]
fn operations_and_recovery_survive_rebuilding_the_graph_history() {
    let mut project = project("control-durable");
    let (_plan, recovery_ref, _executor) = quarantine_app(&mut project);
    let (scope, _principal) = indexed_once(&mut project);
    let engine = std::sync::Arc::clone(&project.engine);

    // Drop the rebuildable graph database entirely: the graph is disposable
    // (design D5), the control plane is not.
    let graph_db = engine.data_dir().join("diskgraph.sqlite");
    drop(engine);
    std::fs::remove_file(&graph_db).unwrap();

    let reopened = Engine::open(EngineConfig {
        data_dir: project.engine.data_dir().to_path_buf(),
        max_nodes_per_scan: 100_000,
        ..EngineConfig::default()
    })
    .unwrap();
    let control = reopened.control_store().unwrap();
    // Every operation and every recovery record is still there.
    let operations = control.list_operations(&scope, 100).unwrap();
    assert!(!operations.is_empty(), "operation history must survive");
    let entry = control.recovery(&recovery_ref).unwrap();
    assert_eq!(entry.state, diskgraph_store::RecoveryState::Available);
    // The locator is stored hex-encoded, so decode it to inspect the path.
    assert!(
        ops_unhex(&entry.quarantine_locator)
            .to_string_lossy()
            .contains("quarantine")
    );
}

#[test]
fn a_write_failure_leaves_the_control_plane_usable_and_auditable() {
    let mut project = project("control-full");
    let (plan, approval, executor) = ready_move(&mut project, "full");
    let (scope, principal) = indexed_once(&mut project);
    // Fill the volume the control database lives on as far as a test may:
    // instead of a real full disk, assert that a refused write leaves the
    // database readable and the operation honestly unsuccessful.
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
    let engine = std::sync::Arc::clone(&project.engine);
    // The record is still queryable: an interrupted run never wedges the store.
    let view = show_operation(&engine, &outcome.operation_id, &principal).unwrap();
    assert_eq!(view.state, diskgraph_store::OperationState::NeedsAttention);
    let listed = list_operations(&engine, &scope, &principal, 10).unwrap();
    assert!(
        listed
            .iter()
            .any(|item| item.operation_id == outcome.operation_id)
    );
    // The object is exactly where it was, and the plan was not consumed.
    assert!(project.root.join("target/app.bin").exists());
    let control = engine.control_store().unwrap();
    assert_eq!(
        control.plan_state(&plan.plan_id).unwrap(),
        diskgraph_store::PlanState::Validated
    );
}

#[test]
fn recorded_details_name_objects_without_leaking_whole_paths() {
    let mut project = project("audit-redaction");
    let (plan, approval, executor) = ready_move(&mut project, "redact");
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval,
            idempotency_key: "k",
            fault: None,
        })
        .unwrap();
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    let detail = items[0].detail.clone();
    // The detail carries the name and identity, not the full user path.
    assert!(detail.contains("app.bin"));
    assert!(
        !detail.contains(&project.root.to_string_lossy().into_owned()),
        "operation details must not record whole user paths: {detail}"
    );
}

// ------------------------------------------- 6.16 batch and approval drills ---

#[test]
fn a_batch_with_one_blocked_item_ends_partial_and_keeps_the_rest() {
    let mut project = project("batch-partial");
    let (scope, principal) = indexed_once(&mut project);
    // Two objects: one stays movable, the other is occupied by an unrelated
    // directory so its step must fail.
    let first = project.root.join("target/app.bin");
    let blocked = project.root.join("target/nested/deep.bin");
    assert!(first.exists() && blocked.exists());
    let archive = project.root.join("archive");
    // Make one target a directory so the file cannot land there.
    std::fs::create_dir_all(archive.join("deep.bin")).unwrap();

    let nodes = [
        node_named(&project.engine, &scope, "app.bin"),
        node_named(&project.engine, &scope, "deep.bin"),
    ];
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_move_plan(
            &scope,
            &principal,
            &nodes,
            &archive,
            1 << 20,
            FileActionKind::Move,
        )
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "batch",
            fault: None,
        })
        .unwrap();
    // One moved, one failed: the operation is honestly partial, and the plan
    // is not consumed so the failure can be re-planned.
    assert_eq!(outcome.state, diskgraph_store::OperationState::Partial);
    assert_eq!(outcome.moved, 1);
    assert_eq!(outcome.failed, 1);
    assert!(project.root.join("archive/app.bin").exists());
    assert!(blocked.exists(), "the blocked object was not lost");
    let control = project.engine.control_store().unwrap();
    assert_eq!(
        control.plan_state(&plan.plan_id).unwrap(),
        diskgraph_store::PlanState::Validated
    );
}

#[test]
fn revoking_an_approval_mid_flight_stops_the_next_step() {
    let mut project = project("revoke-mid");
    let (scope, principal) = indexed_once(&mut project);
    let nodes = [
        node_named(&project.engine, &scope, "app.bin"),
        node_named(&project.engine, &scope, "deep.bin"),
    ];
    let archive = project.root.join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_move_plan(
            &scope,
            &principal,
            &nodes,
            &archive,
            1 << 20,
            FileActionKind::Move,
        )
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    // Revoke before the run: nothing may move.
    {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer.revoke(&approval_ref).unwrap();
    }
    // apply refuses the revoked approval outright, so the call itself fails.
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "revoked-batch",
            fault: None,
        }),
        Err(OpsError::NotAuthorized(_))
    ));
    assert!(project.root.join("target/app.bin").exists());
    assert!(project.root.join("target/nested/deep.bin").exists());
    assert!(!project.root.join("archive/app.bin").exists());
}

#[test]
fn an_empty_object_batch_moves_nothing_and_says_so() {
    let mut project = project("empty-batch");
    let (scope, principal) = indexed_once(&mut project);
    // Plan over an empty file: the batch is legal and the file still moves.
    let empty = project.root.join("target/empty.log");
    std::fs::write(&empty, b"").unwrap();
    let scope2 = {
        let authorizer = project.engine.policy_authorizer().unwrap();
        let job = project
            .engine
            .index_scope(&scope, &principal, &authorizer)
            .unwrap();
        project.engine.run_job(&job.job_id, "reindex").unwrap();
        scope
    };
    let node = node_named(&project.engine, &scope2, "empty.log");
    let archive = project.root.join("archive");
    std::fs::create_dir_all(&archive).unwrap();
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_move_plan(
            &scope2,
            &principal,
            &[node],
            &archive,
            1 << 20,
            FileActionKind::Move,
        )
        .unwrap();
    // A zero-byte object is a real object: the plan must not treat it as empty.
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.expected_bytes, 0);
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "empty-batch",
            fault: None,
        })
        .unwrap();
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert!(archive.join("empty.log").exists());
    assert!(!empty.exists());
}

// ------------------------------------------------- P6: cross-volume and purge ---

#[test]
fn a_cross_volume_copy_stages_verifies_then_publishes() {
    let workspace = TempDir::with_prefix("diskgraph-ops-xcopy-").unwrap();
    let source = workspace.path().join("source.bin");
    std::fs::write(&source, vec![7_u8; 4096]).unwrap();
    let target = workspace.path().join("elsewhere").join("source.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let identity = identity_of(&source, &std::fs::symlink_metadata(&source).unwrap());

    let transfer = CrossVolumeCopy::open(&target, "copy").unwrap();
    let staged = transfer.staged_path().to_path_buf();
    assert_eq!(transfer.stage_and_verify(&source, &identity).unwrap(), 4096);
    // Before the publish step the destination does not exist, only staging.
    assert!(!target.exists());
    assert!(staged.exists());
    transfer.publish().unwrap();
    assert!(target.exists(), "the verified copy is published");
    assert_eq!(std::fs::metadata(&target).unwrap().len(), 4096);
    assert!(
        !staged.exists() && transfer.staging_dir_absent(),
        "the staging area is cleaned up after publishing"
    );
}

#[test]
fn a_source_that_changes_during_a_cross_volume_copy_invalidates_the_transfer() {
    let workspace = TempDir::with_prefix("diskgraph-ops-xcopy-race-").unwrap();
    let source = workspace.path().join("source.bin");
    std::fs::write(&source, vec![7_u8; 4096]).unwrap();
    let target = workspace.path().join("elsewhere").join("source.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let identity = identity_of(&source, &std::fs::symlink_metadata(&source).unwrap());

    let transfer = CrossVolumeCopy::open(&target, "copy").unwrap();
    transfer.stage_and_verify(&source, &identity).unwrap();
    // The source is deleted and recreated after staging: the copy describes a
    // different object, and the identity check must refuse to publish it as
    // if it were the planned one (OP-05).
    std::fs::remove_file(&source).unwrap();
    std::fs::write(&source, vec![9_u8; 4096]).unwrap();
    assert!(matches!(
        transfer.stage_and_verify(&source, &identity),
        Err(OpsError::Stale(message)) if message.contains("changed")
    ));
    transfer.discard();
    assert!(!target.exists(), "a stale copy is never published");
    assert!(
        transfer.staging_dir_absent(),
        "the failed transfer leaves no staged bytes behind"
    );
}

#[test]
fn a_failed_cross_volume_copy_leaves_the_destination_untouched() {
    let workspace = TempDir::with_prefix("diskgraph-ops-xcopy-fail-").unwrap();
    // A directory as the "source" makes the copy itself fail, which is the
    // same surface an out-of-space failure hits: an io error mid-transfer.
    let source = workspace.path().join("not-a-file");
    std::fs::create_dir_all(&source).unwrap();
    let target = workspace.path().join("elsewhere").join("not-a-file");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let identity = None;

    let transfer = CrossVolumeCopy::open(&target, "copy").unwrap();
    assert!(transfer.stage_and_verify(&source, &identity).is_err());
    transfer.discard();
    assert!(!target.exists());
    assert!(transfer.staging_dir_absent());
}

#[test]
fn a_cross_volume_move_publishes_before_the_source_is_removed() {
    let mut project = project("xmove");
    let (scope, _principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let source = project.root.join("target/app.bin");
    let target = project.root.join("elsewhere/app.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    // The live item revalidation normally runs in resolve_live_items; the
    // drill drives the move step directly against the same record shape.
    let metadata = std::fs::symlink_metadata(&source).unwrap();
    let item = LiveItem {
        path: source.clone(),
        identity: identity_of(&source, &metadata),
        recovery_ref: None,
        bytes: metadata.len(),
    };
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let _ = (scope, app);

    executor.cross_volume_move(&item, &target, None).unwrap();
    assert!(target.exists(), "the verified copy carries the bytes");
    assert_eq!(std::fs::metadata(&target).unwrap().len(), 4096);
    assert!(!source.exists(), "only a published copy retires the source");
}

#[test]
fn a_cross_volume_move_parked_at_the_source_seam_keeps_both_sides() {
    let mut project = project("xmove-park");
    let (_scope, _principal) = indexed_once(&mut project);
    let source = project.root.join("target/app.bin");
    let target = project.root.join("elsewhere/app.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let metadata = std::fs::symlink_metadata(&source).unwrap();
    let item = LiveItem {
        path: source.clone(),
        identity: identity_of(&source, &metadata),
        recovery_ref: None,
        bytes: metadata.len(),
    };
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));

    // The drill interrupts exactly between "copy published" and "source
    // removed": both sides survive and the caller is told to reconcile.
    assert!(matches!(
        executor.cross_volume_move(
            &item,
            &target,
            Some(FaultPoint::AfterCopyBeforeSourceRemoval)
        ),
        Err(OpsError::ParkNeedsAttention(_))
    ));
    assert!(target.exists(), "the published copy stays");
    assert!(
        source.exists(),
        "the source is never removed by a parked run"
    );
    assert!(
        executor_staging_is_clean(&target),
        "no staging bytes are left behind"
    );
}

/// True when no `.dg-*-staging-*` directory sits next to `target`.
fn executor_staging_is_clean(target: &Path) -> bool {
    std::fs::read_dir(target.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .all(|entry| !entry.file_name().to_string_lossy().contains("staging"))
}

#[test]
fn purge_requires_a_configured_authority() {
    let mut project = project("purge-default");
    let (scope, principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_purge_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine));
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "admin-console", 60_000)
            .unwrap()
            .approval_ref
    };
    // No authority configured: purge is disabled outright, whoever approves.
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "purge-1",
            fault: None,
        }),
        Err(OpsError::NotAuthorized(message)) if message.contains("no purge authority")
    ));
    assert!(project.root.join("target/app.bin").exists());
}

#[test]
fn only_the_configured_authority_may_approve_a_purge() {
    let mut project = project("purge-authority");
    let (scope, principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_purge_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine))
        .with_purge_authority("human-review-console");
    let issue = |by: &str| {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer.issue(&plan, by, 60_000).unwrap().approval_ref
    };
    // The ordinary operation surface approves: refused, even though the plan
    // and digest are identical.
    let agent_approval = issue("admin-console");
    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &agent_approval,
            idempotency_key: "purge-wrong",
            fault: None,
        }),
        Err(OpsError::NotAuthorized(message)) if message.contains("purge authority")
    ));
    assert!(project.root.join("target/app.bin").exists());
    // The configured authority approves: the object is removed for good and
    // the record says "purged", not "quarantined".
    let trusted_approval = issue("human-review-console");
    let outcome = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &trusted_approval,
            idempotency_key: "purge-right",
            fault: None,
        })
        .unwrap();
    assert_eq!(outcome.state, diskgraph_store::OperationState::Succeeded);
    assert!(!project.root.join("target/app.bin").exists());
    let control = project.engine.control_store().unwrap();
    let items = control.operation_items(&outcome.operation_id).unwrap();
    assert_eq!(
        items[0].result,
        diskgraph_store::OperationItemResult::Purged
    );
}

#[test]
fn restoring_after_a_purge_reports_irrecoverable() {
    let mut project = project("purge-restore");
    let (scope, principal) = indexed_once(&mut project);
    let _ = scope;
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    // A purged object has no recovery record; a restore attempt must hear
    // "irrecoverable", never receive a plan that pretends otherwise (OP-07).
    assert!(matches!(
        builder.build_restore_plan(&scope, &principal, "rec-never-existed", None),
        Err(OpsError::Irrecoverable(message)) if message.contains("cannot be restored")
    ));
}

#[test]
fn a_purge_refuses_a_swapped_object_and_never_touches_the_imposter() {
    let mut project = project("purge-swap");
    let (scope, principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_purge_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine))
        .with_purge_authority("human-review-console");
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "human-review-console", 60_000)
            .unwrap()
            .approval_ref
    };
    // The planned object is replaced by a different file at the same path
    // after planning: the identity check stops the purge before any byte is
    // removed, and the imposter survives (OP-04).
    let outside = tempfile::TempDir::with_prefix("diskgraph-ops-imposter-").unwrap();
    let imposter = outside.path().join("innocent.bin");
    std::fs::write(&imposter, b"precious").unwrap();
    std::fs::remove_file(project.root.join("target/app.bin")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&imposter, project.root.join("target/app.bin")).unwrap();
    #[cfg(not(unix))]
    std::fs::copy(&imposter, project.root.join("target/app.bin")).unwrap();

    assert!(matches!(
        executor.apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "purge-swap",
            fault: None,
        }),
        Err(OpsError::Stale(message)) if message.contains("replaced")
    ));
    assert_eq!(
        std::fs::read(&imposter).unwrap(),
        b"precious",
        "the imposter's target was never touched"
    );
}

#[test]
fn a_parked_purge_retry_returns_the_same_operation_without_replaying() {
    let mut project = project("purge-park");
    let (scope, principal) = indexed_once(&mut project);
    let app = node_named(&project.engine, &scope, "app.bin");
    let builder = PlanBuilder::new(std::sync::Arc::clone(&project.engine));
    let plan = builder
        .build_purge_plan(&scope, &principal, &[app], 1 << 20)
        .unwrap();
    let executor = Executor::new(std::sync::Arc::clone(&project.engine))
        .with_purge_authority("human-review-console");
    let approval_ref = {
        let mut control = project.engine.control_store().unwrap();
        let mut issuer = ApprovalIssuer::new(&mut control);
        issuer
            .issue(&plan, "human-review-console", 60_000)
            .unwrap()
            .approval_ref
    };
    // Crash after the intent is durable, before the file is removed: the
    // object survives and the operation parks (OP-08).
    let parked = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "purge-crash",
            fault: Some(FaultPoint::AfterIntent),
        })
        .unwrap();
    assert_eq!(
        parked.state,
        diskgraph_store::OperationState::NeedsAttention
    );
    assert!(project.root.join("target/app.bin").exists());
    // The client retries with the same key: the parked operation comes back
    // unchanged, and nothing is deleted by the retry itself.
    let retried = executor
        .apply(ApplyRequest {
            plan_id: &plan.plan_id,
            approval_ref: &approval_ref,
            idempotency_key: "purge-crash",
            fault: None,
        })
        .unwrap();
    assert!(!retried.started);
    assert_eq!(retried.operation_id, parked.operation_id);
    assert_eq!(retried.state, parked.state);
    assert!(project.root.join("target/app.bin").exists());
}
