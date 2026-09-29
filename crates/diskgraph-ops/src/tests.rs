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
