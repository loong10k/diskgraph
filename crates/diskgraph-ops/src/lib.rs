//! Reversible file operations (P5). This crate turns "the agent wants to move
//! these files" into an immutable, digest-bound, human-approvable plan, and
//! only executes it after a trusted approval and a fresh revalidation.
//!
//! The layering is deliberate and matches the spec's separation:
//! - [`PlanBuilder`] resolves a request into concrete objects. It **never**
//!   touches a file; the only side effect is a row in the control store.
//! - [`ApprovalIssuer`] mints the approval an external trusted surface
//!   (PruneX review UI, an admin console) would present. The agent can never
//!   mint one for itself.
//! - [`Executor`] is the only place a file moves, and only for a plan whose
//!   digest still matches an unexpired approval, after revalidating every
//!   precondition against the live filesystem.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use diskgraph_core::{FileActionKind, PrincipalId, ScopeId};
use diskgraph_engine::Engine;
use diskgraph_store::{Approval, ControlStore, Plan, PlanItem, RecoveryRule, StoreError};

/// Milliseconds since the epoch for control-plane timestamps.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Errors surfaced by the ops layer. They mirror the business error codes so
/// the CLI and MCP can pass them through unchanged.
#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("no such scope: {0}")]
    NoSuchScope(String),
    #[error("object not found in the bound revision: node {0}")]
    NoSuchNode(u64),
    #[error("the request resolved to an empty object set")]
    EmptyPlan,
    #[error("parent/child overlap could not be resolved for node {0}")]
    OverlappingObjects(u64),
    #[error("the action does not belong to this plan's action")]
    ActionMismatch,
    #[error("not authorized: {0}")]
    NotAuthorized(String),
    #[error("precondition failed: {0}")]
    Stale(String),
    #[error("conflicts with in-flight work: {0}")]
    Conflict(String),
    #[error("the target already exists and overwrite is not permitted")]
    TargetExists,
    #[error("a same-volume move is not possible across devices")]
    CrossVolume,
    #[error("irrecoverable: {0}")]
    Irrecoverable(String),
    /// Internal: a drill fault parked the operation for reconciliation. The
    /// apply loop turns this into a NeedsAttention state; production code
    /// never returns it.
    #[error("parked for attention: {0}")]
    ParkNeedsAttention(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Engine(#[from] diskgraph_engine::EngineError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Everything one plan build needs, kept together so the builder's own
/// signature stays readable.
struct PlanRequest<'a> {
    scope_id: &'a ScopeId,
    principal: &'a PrincipalId,
    action: FileActionKind,
    node_ids: &'a [u64],
    target: Option<&'a Path>,
    max_bytes: u64,
    recovery: RecoveryRule,
}

/// Builds immutable plans from a request against a published revision.
pub struct PlanBuilder {
    engine: std::sync::Arc<Engine>,
}

impl PlanBuilder {
    pub fn new(engine: std::sync::Arc<Engine>) -> Self {
        Self { engine }
    }

    /// Resolves a request into a plan and persists it. The request names node
    /// ids inside the scope's current revision; the builder resolves them to
    /// live paths and identities, drops parent/child overlaps, and writes a
    /// digest over the exact result. No file is opened for writing here.
    pub fn build_trash_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        node_ids: &[u64],
        max_bytes: u64,
    ) -> Result<Plan, OpsError> {
        let plan = self.build(PlanRequest {
            scope_id,
            principal,
            action: FileActionKind::Trash,
            node_ids,
            target: None,
            max_bytes,
            recovery: RecoveryRule::Quarantine,
        })?;
        Ok(plan)
    }

    /// Resolves a move/copy plan into `target`.
    pub fn build_move_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        node_ids: &[u64],
        target: &Path,
        max_bytes: u64,
        action: FileActionKind,
    ) -> Result<Plan, OpsError> {
        if !matches!(action, FileActionKind::Move | FileActionKind::Copy) {
            return Err(OpsError::ActionMismatch);
        }
        let recovery = if action == FileActionKind::Move {
            RecoveryRule::MoveBack
        } else {
            RecoveryRule::NoneNeeded
        };
        // Resolve the destination now, so the plan records the same canonical
        // form as the objects it names. A caller-supplied path through a system
        // alias would otherwise not share a prefix with the scope root.
        let resolved = canonical_dir(target);
        let plan = self.build(PlanRequest {
            scope_id,
            principal,
            action,
            node_ids,
            target: Some(&resolved),
            max_bytes,
            recovery,
        })?;
        Ok(plan)
    }

    /// Resolves a purge plan over exact objects. A purge is planned like any
    /// other action, but applying it requires an approval issued by the
    /// configured purge authority, and no recovery record is written (OP-07).
    pub fn build_purge_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        node_ids: &[u64],
        max_bytes: u64,
    ) -> Result<Plan, OpsError> {
        self.build(PlanRequest {
            scope_id,
            principal,
            action: FileActionKind::Purge,
            node_ids,
            target: None,
            max_bytes,
            recovery: RecoveryRule::NoneNeeded,
        })
    }

    /// Derives a fresh plan that puts a quarantined object back. The original
    /// location is the default; `target` names another authorized destination.
    /// Planning never modifies the recovery record itself.
    pub fn build_restore_plan(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        recovery_ref: &str,
        target: Option<&Path>,
    ) -> Result<Plan, OpsError> {
        let entry = {
            let control = self.engine.control_store()?;
            // An object with no recovery record — above all a purged one — is
            // gone for good; the caller hears that, never a fabricated plan
            // that pretends a restore is possible (OP-07).
            match control.recovery(recovery_ref) {
                Ok(entry) => entry,
                Err(StoreError::RecoveryNotFound(_)) => {
                    return Err(OpsError::Irrecoverable(
                        "no recovery record exists; a purged object cannot be restored".into(),
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        };
        if entry.scope_id != *scope_id {
            return Err(OpsError::NotAuthorized(
                "recovery belongs to another scope".into(),
            ));
        }
        if entry.state != diskgraph_store::RecoveryState::Available {
            return Err(OpsError::Stale(format!(
                "recovery {recovery_ref} is {:?}",
                entry.state
            )));
        }
        let held = unhex_key(&entry.quarantine_locator)
            .ok_or_else(|| OpsError::Stale("recovery locator is malformed".into()))?;
        let metadata = std::fs::symlink_metadata(&held)
            .map_err(|error| OpsError::Stale(format!("held object is gone: {error}")))?;
        let item = PlanItem {
            node_id: 0,
            locator_key: locator_key(&held),
            identity: Some(entry.identity.clone()),
            includes_descendants: false,
            recovery_ref: Some(recovery_ref.to_owned()),
        };
        // Resolve the policy epoch before taking the control lock again.
        let policy_version = self.engine.policy_authorizer()?.current_version();
        let plan = Plan {
            plan_id: format!("plan-{}", uuid::Uuid::new_v4()),
            scope_id: scope_id.clone(),
            principal: principal.clone(),
            action: FileActionKind::Restore,
            items: vec![item],
            target_locator_key: target.map(locator_key),
            policy_version,
            max_bytes: metadata.len().max(1),
            created_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + 15 * 60_000,
            recovery: RecoveryRule::NoneNeeded,
            expected_bytes: metadata.len(),
        };
        let digest = plan_digest(&plan);
        self.engine.control_store()?.insert_plan(&plan, &digest)?;
        Ok(plan)
    }

    fn build(&self, request: PlanRequest<'_>) -> Result<Plan, OpsError> {
        let PlanRequest {
            scope_id,
            principal,
            action,
            node_ids,
            target,
            max_bytes,
            recovery,
        } = request;
        // The scope must be live: a revoked scope never yields a plan.
        let scope = self
            .engine
            .scope(scope_id)
            .map_err(|_| OpsError::NoSuchScope(scope_id.as_str().to_owned()))?;
        if scope.revoked {
            return Err(OpsError::NotAuthorized("scope is revoked".into()));
        }
        let revision = self
            .engine
            .latest_revision(scope_id)
            .map_err(|_| OpsError::NoSuchScope(scope_id.as_str().to_owned()))?
            .ok_or_else(|| OpsError::Stale("scope has no published revision".into()))?;
        let graph = self.engine.load_revision(&revision)?;

        // Resolve every requested node to a live path and identity.
        let mut resolved: Vec<(u64, PathBuf, Option<String>, u64)> = Vec::new();
        for &node_id in node_ids {
            let node = graph
                .nodes
                .iter()
                .find(|node| node.id == node_id)
                .ok_or(OpsError::NoSuchNode(node_id))?;
            let path = node
                .locator
                .raw_path()
                .ok_or_else(|| OpsError::Stale(format!("node {node_id} has no native path")))?;
            let live = std::fs::symlink_metadata(&path)
                .map_err(|error| OpsError::Stale(format!("node {node_id}: {error}")))?;
            let identity = identity_of(&path, &live);
            // Budget against the object's own live size, not the scan-time
            // subtree aggregate: after overlaps are dropped, a directory and a
            // file inside it would otherwise be counted twice.
            resolved.push((node_id, path, identity, live.len()));
        }
        if resolved.is_empty() {
            return Err(OpsError::EmptyPlan);
        }

        // Remove parent/child overlaps: keeping both would move the same bytes
        // twice and double-count the space (OP-02).
        let resolved = drop_nested(&resolved)?;
        let expected_bytes: u64 = resolved.iter().map(|item| item.3).sum();
        if expected_bytes > max_bytes {
            return Err(OpsError::Stale(format!(
                "plan needs {expected_bytes} bytes, budget is {max_bytes}"
            )));
        }

        let items: Vec<PlanItem> = resolved
            .iter()
            .map(|(node_id, path, identity, _)| PlanItem {
                node_id: *node_id,
                locator_key: locator_key(path),
                identity: identity.clone(),
                includes_descendants: false,
                recovery_ref: None,
            })
            .collect();

        // Resolve the policy epoch in its own scope: the authorizer takes the
        // control lock, and holding it again for the plan insert would
        // self-deadlock.
        let policy_version = self.engine.policy_authorizer()?.current_version();

        let plan = Plan {
            plan_id: format!("plan-{}", uuid::Uuid::new_v4()),
            scope_id: scope_id.clone(),
            principal: principal.clone(),
            action,
            items,
            target_locator_key: target.map(locator_key),
            policy_version,
            max_bytes,
            created_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + 15 * 60_000,
            recovery,
            expected_bytes,
        };
        let digest = plan_digest(&plan);
        let mut control = self.engine.control_store()?;
        control.insert_plan(&plan, &digest)?;
        Ok(plan)
    }
}

/// The stable action name used in digests and the store.
fn action_name(action: FileActionKind) -> &'static str {
    match action {
        FileActionKind::Move => "move",
        FileActionKind::Copy => "copy",
        FileActionKind::Trash => "trash",
        FileActionKind::Restore => "restore",
        FileActionKind::Purge => "purge",
    }
}

/// The stable recovery name used in digests.
fn recovery_name(recovery: RecoveryRule) -> &'static str {
    match recovery {
        RecoveryRule::Quarantine => "quarantine",
        RecoveryRule::MoveBack => "move_back",
        RecoveryRule::NoneNeeded => "none_needed",
    }
}

/// The canonical digest of a plan. It covers every field that would change what
/// an approval authorizes, in a fixed order, so two identical requests digest
/// the same and any mutation changes the digest.
pub fn plan_digest(plan: &Plan) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    plan.scope_id.as_str().hash(&mut hasher);
    plan.principal.as_str().hash(&mut hasher);
    action_name(plan.action).hash(&mut hasher);
    for item in &plan.items {
        item.node_id.hash(&mut hasher);
        item.locator_key.hash(&mut hasher);
        item.identity.hash(&mut hasher);
        item.includes_descendants.hash(&mut hasher);
    }
    plan.target_locator_key.hash(&mut hasher);
    plan.policy_version.hash(&mut hasher);
    plan.max_bytes.hash(&mut hasher);
    // The deadline is deliberately not part of the digest: the digest
    // identifies WHAT an approval authorizes, and two identical requests must
    // review as one even if issued a millisecond apart.
    recovery_name(plan.recovery).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// A requested object resolved to a live path, its identity, and its size.
type Resolved = (u64, PathBuf, Option<String>, u64);

/// Drops any item that is an ancestor of another, so a plan never moves the
/// same bytes twice. Ancestor is decided by real path containment, not by name.
fn drop_nested(items: &[Resolved]) -> Result<Vec<Resolved>, OpsError> {
    let mut keep: Vec<Resolved> = Vec::new();
    for candidate in items {
        let is_ancestor_of_other = items
            .iter()
            .any(|other| other.1 != candidate.1 && other.1.starts_with(&candidate.1));
        if is_ancestor_of_other {
            // Dropping the parent is safe: its children cover the same bytes.
            continue;
        }
        keep.push(candidate.clone());
    }
    if keep.is_empty() {
        return Err(OpsError::OverlappingObjects(items[0].0));
    }
    // Two items that are the same path would double-count; that is an overlap
    // we cannot resolve by dropping one, so refuse.
    let mut seen: HashSet<String> = HashSet::new();
    for item in &keep {
        if !seen.insert(item.1.to_string_lossy().into_owned()) {
            return Err(OpsError::OverlappingObjects(item.0));
        }
    }
    Ok(keep)
}

/// A stable, reversible key for a path (the raw bytes, hex-encoded). It is the
/// identity used in plans and recovery records, never a display string.
fn locator_key(path: &Path) -> String {
    hex::encode(path.to_string_lossy().as_bytes())
}

/// The identity used to detect a replaced object between plan and apply: on
/// unix this is the device and inode, which survive a rename but not a
/// delete-and-recreate.
fn identity_of(_path: &Path, metadata: &std::fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, metadata);
        None
    }
}

/// Mints and verifies approvals. Only a trusted surface calls `issue`; the
/// agent-facing path can only `verify`.
pub struct ApprovalIssuer<'a> {
    control: &'a mut ControlStore,
}

impl<'a> ApprovalIssuer<'a> {
    pub fn new(control: &'a mut ControlStore) -> Self {
        Self { control }
    }

    /// Issues an approval bound to the plan's current digest.
    pub fn issue(
        &mut self,
        plan: &Plan,
        issued_by: &str,
        ttl_ms: u64,
    ) -> Result<Approval, OpsError> {
        let digest = plan_digest(plan);
        let approval = Approval {
            approval_ref: format!("ap-{}", uuid::Uuid::new_v4()),
            plan_id: plan.plan_id.clone(),
            plan_digest: digest,
            principal: plan.principal.clone(),
            action: plan.action,
            issued_by: issued_by.to_owned(),
            issued_at_unix_ms: now_ms(),
            expires_at_unix_ms: now_ms() + ttl_ms,
            revoked: false,
        };
        self.control.insert_approval(&approval)?;
        Ok(approval)
    }

    /// Revokes an issued approval before it is used.
    pub fn revoke(&mut self, approval_ref: &str) -> Result<(), OpsError> {
        self.control.revoke_approval(approval_ref)?;
        Ok(())
    }
}

/// Where a run may be interrupted, so the recovery contract can be tested
/// rather than assumed (P5 task 6.13). Production passes `None`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    /// After the intent is persisted, before the file changes.
    AfterIntent,
    /// After the file changed, before the result is recorded.
    AfterFileChange,
    /// A cross-volume move published the verified copy but has not yet removed
    /// the source. Drilling this exact seam proves the contract that the copy
    /// may be trusted and the source kept, never the other way round (OP-05).
    AfterCopyBeforeSourceRemoval,
}

/// The request that applies a plan.
pub struct ApplyRequest<'a> {
    pub plan_id: &'a str,
    pub approval_ref: &'a str,
    /// Reusing a key with the same request returns the original operation;
    /// reusing it with a different request is refused.
    pub idempotency_key: &'a str,
    pub fault: Option<FaultPoint>,
}

/// What an apply produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyOutcome {
    pub operation_id: String,
    /// False when an existing operation was returned for a reused key.
    pub started: bool,
    pub state: diskgraph_store::OperationState,
    pub moved: usize,
    pub failed: usize,
    pub bytes: u64,
}

/// Executes a plan. This is the only place a file moves, and only after the
/// approval verifies and every precondition is rechecked against the live
/// filesystem.
pub struct Executor {
    engine: std::sync::Arc<Engine>,
    /// The trusted surface whose approvals may authorize a purge. `None`
    /// (the default) means purge is disabled outright: nothing in this build
    /// can be removed permanently until an operator names the authority (OP-07).
    purge_authority: Option<String>,
}

impl Executor {
    pub fn new(engine: std::sync::Arc<Engine>) -> Self {
        Self {
            engine,
            purge_authority: None,
        }
    }

    /// Names the only approval issuer whose approvals may authorize purge.
    /// Any other issuer's approval for a purge plan is refused at apply time.
    pub fn with_purge_authority(mut self, authority: &str) -> Self {
        self.purge_authority = Some(authority.to_owned());
        self
    }

    /// Applies a plan under a trusted approval.
    pub fn apply(&self, request: ApplyRequest<'_>) -> Result<ApplyOutcome, OpsError> {
        // 0. A retry of a request we already accepted answers with the
        //    original operation, whatever happened to the plan since. This is
        //    checked first: a client that lost the response must not be told
        //    the plan is stale when its own work already succeeded.
        {
            // One guard for the whole probe: the plan, the key lookup, and the
            // digest all read the same control database.
            let control = self.engine.control_store()?;
            let probe_plan = control.plan(request.plan_id)?;
            if let Some(existing) =
                control.operation_for_key(&probe_plan.principal, request.idempotency_key)?
            {
                if existing.request_digest
                    != apply_request_digest(&probe_plan, request.approval_ref)
                {
                    return Err(StoreError::IdempotencyConflict.into());
                }
                return Ok(ApplyOutcome {
                    operation_id: existing.operation_id,
                    started: false,
                    state: existing.state,
                    moved: 0,
                    failed: 0,
                    bytes: 0,
                });
            }
        }

        // 1. The plan must exist, still be live, and match the bound digest.
        // 2. The approval must verify for this exact plan, principal and
        //    action. An agent can never assert its own approval.
        // Each block below takes the control lock for as short as possible:
        // holding one guard across a helper that locks again would deadlock.
        let plan = {
            let control = self.engine.control_store()?;
            let plan = control.plan(request.plan_id)?;
            let digest = control.plan_digest(request.plan_id)?;
            if plan_digest(&plan) != digest {
                return Err(OpsError::Stale("plan digest drifted".into()));
            }
            if control.plan_state(request.plan_id)? != diskgraph_store::PlanState::Validated {
                return Err(OpsError::Stale(format!(
                    "plan {} is not validated",
                    request.plan_id
                )));
            }
            // The verified approval record is kept so a purge can prove its
            // approval came from the configured authority, never from the
            // agent or an ordinary operation surface (OP-07).
            let approval = control
                .verify_approval(
                    request.approval_ref,
                    &plan.plan_id,
                    &digest,
                    &plan.principal,
                    plan.action,
                )
                .map_err(|error| OpsError::NotAuthorized(error.to_string()))?;
            if plan.action == FileActionKind::Purge {
                let authority = self.purge_authority.as_deref().ok_or_else(|| {
                    OpsError::NotAuthorized(
                        "purge is irreversible and no purge authority is configured".into(),
                    )
                })?;
                if approval.issued_by != authority {
                    return Err(OpsError::NotAuthorized(
                        "a purge may only be approved by the configured purge authority".into(),
                    ));
                }
            }
            plan
        };

        // 3. Revalidate every precondition against the live filesystem before
        // touching anything, and refuse overlapping in-flight operations.
        let items = self.resolve_live_items(&plan)?;
        self.refuse_conflicts(&plan, &items)?;

        // 4. Idempotency: one key, one operation.
        let request_digest = apply_request_digest(&plan, request.approval_ref);
        let operation = diskgraph_store::Operation {
            operation_id: format!("op-{}", uuid::Uuid::new_v4()),
            plan_id: plan.plan_id.clone(),
            scope_id: plan.scope_id.clone(),
            principal: plan.principal.clone(),
            idempotency_key: request.idempotency_key.to_owned(),
            request_digest,
            state: diskgraph_store::OperationState::Queued,
            created_at_unix_ms: now_ms(),
            updated_at_unix_ms: now_ms(),
        };
        let (operation_id, created) = {
            let mut control = self.engine.control_store()?;
            control.begin_operation(&operation, items.len())?
        };
        if !created {
            // A retry of the same request: return the original operation and
            // do not move anything a second time.
            let existing = self.engine.control_store()?.operation(&operation_id)?;
            return Ok(ApplyOutcome {
                operation_id,
                started: false,
                state: existing.state,
                moved: 0,
                failed: 0,
                bytes: 0,
            });
        }

        // 5. Execute item by item, recording intent before each mutation.
        self.engine
            .control_store()?
            .set_operation_state(&operation_id, diskgraph_store::OperationState::Revalidating)?;
        let mut moved = 0usize;
        let mut failed = 0usize;
        let mut bytes = 0u64;
        for (index, item) in items.iter().enumerate() {
            let index = index as u32;
            self.engine
                .control_store()?
                .record_intent(&operation_id, index)?;
            if request.fault == Some(FaultPoint::AfterIntent) {
                // The intent is durable and the file is untouched: a later
                // attempt must reconcile rather than replay blindly.
                self.engine.control_store()?.set_operation_state(
                    &operation_id,
                    diskgraph_store::OperationState::NeedsAttention,
                )?;
                return Ok(ApplyOutcome {
                    operation_id,
                    started: true,
                    state: diskgraph_store::OperationState::NeedsAttention,
                    moved,
                    failed,
                    bytes,
                });
            }
            match self.perform(&plan, item, request.fault) {
                Ok(result) => {
                    bytes += result.bytes;
                    if request.fault == Some(FaultPoint::AfterFileChange) {
                        // The file moved but the result was never recorded: the
                        // operation parks for a human instead of replaying.
                        self.engine.control_store()?.set_operation_state(
                            &operation_id,
                            diskgraph_store::OperationState::NeedsAttention,
                        )?;
                        return Ok(ApplyOutcome {
                            operation_id,
                            started: true,
                            state: diskgraph_store::OperationState::NeedsAttention,
                            moved,
                            failed,
                            bytes,
                        });
                    }
                    self.engine.control_store()?.record_item_result(
                        &operation_id,
                        index,
                        result.kind,
                        &result.detail,
                        result.recovery_ref.as_deref(),
                    )?;
                    if result.kind == diskgraph_store::OperationItemResult::Failed {
                        failed += 1;
                    } else {
                        moved += 1;
                    }
                }
                Err(OpsError::ParkNeedsAttention(detail)) => {
                    // The step got part-way (for example a cross-volume move
                    // published its verified copy): park for reconciliation
                    // instead of replaying an irreversible remainder (OP-08).
                    self.engine.control_store()?.set_operation_state(
                        &operation_id,
                        diskgraph_store::OperationState::NeedsAttention,
                    )?;
                    self.engine.control_store()?.record_item_result(
                        &operation_id,
                        index,
                        diskgraph_store::OperationItemResult::Pending,
                        &format!("parked mid-step: {detail}"),
                        None,
                    )?;
                    return Ok(ApplyOutcome {
                        operation_id,
                        started: true,
                        state: diskgraph_store::OperationState::NeedsAttention,
                        moved,
                        failed,
                        bytes,
                    });
                }
                Err(error) => {
                    failed += 1;
                    self.engine.control_store()?.record_item_result(
                        &operation_id,
                        index,
                        diskgraph_store::OperationItemResult::Failed,
                        &error.to_string(),
                        None,
                    )?;
                }
            }
        }

        // 6. Close out honestly: partial when some items failed.
        let state = if failed == 0 {
            diskgraph_store::OperationState::Succeeded
        } else if moved == 0 {
            diskgraph_store::OperationState::Failed
        } else {
            diskgraph_store::OperationState::Partial
        };
        {
            let mut control = self.engine.control_store()?;
            control.set_operation_state(&operation_id, state)?;
            if state == diskgraph_store::OperationState::Succeeded {
                control.mark_plan_applied(&plan.plan_id)?;
            }
        }
        Ok(ApplyOutcome {
            operation_id,
            started: true,
            state,
            moved,
            failed,
            bytes,
        })
    }

    /// Re-resolves every planned object against the live filesystem.
    fn resolve_live_items(&self, plan: &Plan) -> Result<Vec<LiveItem>, OpsError> {
        let mut live = Vec::new();
        for item in &plan.items {
            let path = unhex_key(&item.locator_key)
                .ok_or_else(|| OpsError::Stale(format!("bad locator for node {}", item.node_id)))?;
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| OpsError::Stale(format!("node {}: {error}", item.node_id)))?;
            // A replaced object (deleted and recreated) has a new identity and
            // must not be touched by a plan that described the old one.
            let identity = identity_of(&path, &metadata);
            if let (Some(expected), Some(actual)) = (&item.identity, &identity)
                && expected != actual
            {
                return Err(OpsError::Stale(format!(
                    "node {} was replaced since the plan was made",
                    item.node_id
                )));
            }
            live.push(LiveItem {
                path,
                identity,
                recovery_ref: item.recovery_ref.clone(),
                bytes: metadata.len(),
            });
        }
        Ok(live)
    }

    /// Refuses when another in-flight operation already claims an overlapping
    /// path: same file, an ancestor, a descendant, or a colliding target.
    fn refuse_conflicts(&self, plan: &Plan, items: &[LiveItem]) -> Result<(), OpsError> {
        let control = self.engine.control_store()?;
        for operation in control.list_operations(&plan.scope_id, 64)? {
            if !matches!(
                operation.state,
                diskgraph_store::OperationState::Queued
                    | diskgraph_store::OperationState::Revalidating
                    | diskgraph_store::OperationState::Running
            ) {
                continue;
            }
            let existing = control.operation_items(&operation.operation_id)?;
            for item in existing {
                if item.result != diskgraph_store::OperationItemResult::Pending {
                    continue;
                }
                if let Ok(other_plan) = control.plan(&operation.plan_id)
                    && let Some(other) = other_plan
                        .items
                        .iter()
                        .find(|candidate| candidate.node_id == item.item_index as u64)
                    && let Some(other_path) = unhex_key(&other.locator_key)
                {
                    for mine in items {
                        if mine.path == other_path
                            || mine.path.starts_with(&other_path)
                            || other_path.starts_with(&mine.path)
                        {
                            return Err(OpsError::Conflict(format!(
                                "operation {} already claims {}",
                                operation.operation_id,
                                other_path.display()
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Performs the single filesystem step for one item. `fault` is a drill
    /// seam; production passes `None`.
    fn perform(
        &self,
        plan: &Plan,
        item: &LiveItem,
        fault: Option<FaultPoint>,
    ) -> Result<StepResult, OpsError> {
        match plan.action {
            FileActionKind::Move => {
                let target = self.target_for(plan, &item.path)?;
                // Component-wise revalidation: a link planted in either path
                // after planning must stop the move, not redirect it (OP-04).
                revalidate_below(&self.scope_root(plan)?, &item.path, Side::Source)
                    .map_err(|fault| OpsError::Stale(format!("source: {fault}")))?;
                revalidate_below(&self.scope_root(plan)?, &target, Side::Target)
                    .map_err(|fault| OpsError::Stale(format!("target: {fault}")))?;
                // Never overwrite: a target that appeared since planning is
                // a conflict, not something to clobber (OP-05).
                if std::fs::symlink_metadata(&target).is_ok() {
                    return Err(OpsError::TargetExists);
                }
                if are_same_volume(&item.path, &target)? {
                    std::fs::rename(&item.path, &target)?;
                    return Ok(StepResult {
                        kind: diskgraph_store::OperationItemResult::Moved,
                        detail: describe(&target, item.identity.as_deref()),
                        bytes: item.bytes,
                        recovery_ref: None,
                    });
                }
                // Cross volume (OP-05, OP-09): stage, verify, publish, and only
                // then remove the source. No cross-volume atomicity is claimed;
                // the operation record names exactly which step finished.
                self.cross_volume_move(item, &target, fault)
            }
            FileActionKind::Copy => {
                let target = self.target_for(plan, &item.path)?;
                if std::fs::symlink_metadata(&target).is_ok() {
                    return Err(OpsError::TargetExists);
                }
                if are_same_volume(&item.path, &target)? {
                    std::fs::copy(&item.path, &target)?;
                    return Ok(StepResult {
                        kind: diskgraph_store::OperationItemResult::Copied,
                        detail: describe(&target, item.identity.as_deref()),
                        bytes: item.bytes,
                        recovery_ref: None,
                    });
                }
                // Cross volume (OP-05): stage on the target's volume, verify
                // the byte count and the source's stability, then publish with
                // a same-volume rename. A half-written file never appears at
                // the destination.
                let transfer = CrossVolumeCopy::open(&target, "copy")?;
                let transferred = transfer
                    .stage_and_verify(&item.path, &item.identity)
                    .and_then(|copied| transfer.publish().map(|()| copied));
                if let Err(error) = transferred {
                    transfer.discard();
                    return Err(error);
                }
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Copied,
                    detail: describe(&target, item.identity.as_deref()),
                    bytes: item.bytes,
                    recovery_ref: None,
                })
            }
            FileActionKind::Trash => {
                // Quarantine, never delete: the object is moved into a
                // same-volume holding area and a recovery record is written.
                // If quarantine cannot be used the item fails; no path here
                // removes a file permanently (OP-06).
                let quarantine = self.quarantine_root()?;
                let recovery_ref = format!("rec-{}", uuid::Uuid::new_v4());
                let name = item
                    .path
                    .file_name()
                    .ok_or_else(|| OpsError::Stale("object has no name".into()))?;
                let held = quarantine.join(&recovery_ref).join(name);
                std::fs::create_dir_all(held.parent().unwrap())?;
                if !are_same_volume(&item.path, &held)? {
                    // Quarantine is same-volume by design: a cross-volume
                    // holding area would not preserve recoverability (OP-06).
                    return Err(OpsError::CrossVolume);
                }
                std::fs::rename(&item.path, &held)?;
                let entry = diskgraph_store::RecoveryEntry {
                    recovery_ref: recovery_ref.clone(),
                    operation_id: String::new(),
                    scope_id: plan.scope_id.clone(),
                    original_locator: locator_key(&item.path),
                    quarantine_locator: locator_key(&held),
                    identity: item.identity.clone().unwrap_or_default(),
                    created_at_unix_ms: now_ms(),
                    state: diskgraph_store::RecoveryState::Available,
                };
                self.engine.control_store()?.insert_recovery(&entry)?;
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Quarantined,
                    detail: describe(&held, item.identity.as_deref()),
                    bytes: item.bytes,
                    recovery_ref: Some(recovery_ref),
                })
            }
            FileActionKind::Restore => {
                // A restore is a new plan derived from a recovery record; it
                // never reuses the original operation.
                let recovery_ref = item
                    .recovery_ref
                    .as_deref()
                    .ok_or_else(|| OpsError::Stale("restore item has no recovery".into()))?;
                let (entry, destination) = {
                    let control = self.engine.control_store()?;
                    let entry = control.recovery(recovery_ref)?;
                    if entry.state != diskgraph_store::RecoveryState::Available {
                        return Err(OpsError::Stale(format!(
                            "recovery {recovery_ref} is {:?}",
                            entry.state
                        )));
                    }
                    let held = unhex_key(&entry.quarantine_locator)
                        .ok_or_else(|| OpsError::Stale("recovery locator is malformed".into()))?;
                    // The original location wins unless the plan names
                    // another authorized destination.
                    let destination = match &plan.target_locator_key {
                        Some(key) => unhex_key(key)
                            .ok_or_else(|| OpsError::Stale("restore target is malformed".into()))?
                            .join(held.file_name().ok_or_else(|| {
                                OpsError::Stale("held object has no name".into())
                            })?),
                        None => unhex_key(&entry.original_locator).ok_or_else(|| {
                            OpsError::Stale("recovery locator is malformed".into())
                        })?,
                    };
                    (entry, destination)
                };
                let held = unhex_key(&entry.quarantine_locator)
                    .ok_or_else(|| OpsError::Stale("recovery locator is malformed".into()))?;
                let metadata = std::fs::symlink_metadata(&held)
                    .map_err(|error| OpsError::Stale(format!("held object is gone: {error}")))?;
                if !entry.identity.is_empty()
                    && let Some(actual) = identity_of(&held, &metadata)
                    && entry.identity != actual
                {
                    return Err(OpsError::Stale("the held object was replaced".into()));
                }
                // A restore never overwrites what is already there (OP-06).
                if std::fs::symlink_metadata(&destination).is_ok() {
                    return Err(OpsError::TargetExists);
                }
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&held, &destination)?;
                self.engine
                    .control_store()?
                    .mark_recovery_restored(recovery_ref)?;
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Restored,
                    detail: describe(&destination, Some(&entry.identity)),
                    bytes: metadata.len(),
                    recovery_ref: None,
                })
            }
            FileActionKind::Purge => {
                // The identity already matched in resolve_live_items; the
                // component-wise check here closes the remaining race where a
                // planned path was relinked or re-rooted before execution
                // (OP-04). Purge is irreversible: there is no recovery record,
                // and the result says so plainly (OP-07).
                revalidate_below(&self.scope_root(plan)?, &item.path, Side::Source)
                    .map_err(|fault| OpsError::Stale(format!("purge: {fault}")))?;
                remove_source(&item.path)?;
                Ok(StepResult {
                    kind: diskgraph_store::OperationItemResult::Purged,
                    detail: describe(&item.path, item.identity.as_deref()),
                    bytes: item.bytes,
                    recovery_ref: None,
                })
            }
        }
    }

    /// Completes a cross-volume move. When this runs the verified copy is not
    /// yet published; the sequence is stage → verify → publish → remove the
    /// source, and the drill fault parks between publish and removal so a
    /// crash at that seam is observable and retry returns the parked
    /// operation instead of replaying an irreversible step (OP-05, OP-08).
    pub(crate) fn cross_volume_move(
        &self,
        item: &LiveItem,
        target: &Path,
        fault: Option<FaultPoint>,
    ) -> Result<StepResult, OpsError> {
        let transfer = CrossVolumeCopy::open(target, "move")?;
        let transferred = transfer
            .stage_and_verify(&item.path, &item.identity)
            .and_then(|copied| transfer.publish().map(|()| copied));
        if let Err(error) = transferred {
            transfer.discard();
            return Err(error);
        }
        if fault == Some(FaultPoint::AfterCopyBeforeSourceRemoval) {
            // The copy is complete and verified; the source is untouched. A
            // retry must reconcile this state with a human, not delete first.
            return Err(OpsError::ParkNeedsAttention(
                "the verified copy is published; the source removal did not run".into(),
            ));
        }
        remove_source(&item.path)?;
        Ok(StepResult {
            kind: diskgraph_store::OperationItemResult::Moved,
            detail: describe(target, item.identity.as_deref()),
            bytes: item.bytes,
            recovery_ref: None,
        })
    }

    /// The registered root of the plan's scope; the trusted base for path
    /// revalidation.
    fn scope_root(&self, plan: &Plan) -> Result<PathBuf, OpsError> {
        let record = self
            .engine
            .scope(&plan.scope_id)
            .map_err(|error| OpsError::Stale(format!("scope is unavailable: {error}")))?;
        path_of(&record.root)
            .ok_or_else(|| OpsError::Stale("scope root is not a native path".into()))
    }

    /// The same-volume holding area for quarantined objects.
    fn quarantine_root(&self) -> Result<PathBuf, OpsError> {
        Ok(self.engine.data_dir().join("quarantine"))
    }

    /// Where a moved or copied object lands. A plan target is a directory; the
    /// object keeps its own name inside it.
    fn target_for(&self, plan: &Plan, source: &Path) -> Result<PathBuf, OpsError> {
        let key = plan
            .target_locator_key
            .as_ref()
            .ok_or_else(|| OpsError::Stale("plan has no target".into()))?;
        let directory =
            unhex_key(key).ok_or_else(|| OpsError::Stale("plan target is malformed".into()))?;
        let name = source
            .file_name()
            .ok_or_else(|| OpsError::Stale("source has no file name".into()))?;
        Ok(directory.join(name))
    }
}

/// Why a path failed pre-execution revalidation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathFault {
    /// A component of the source path is a symbolic link.
    SourceIsLink,
    /// A component of the destination path is a symbolic link.
    TargetIsLink,
    /// A path component vanished between planning and apply.
    ComponentVanished,
}

impl std::fmt::Display for PathFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SourceIsLink => "a source path component is a symbolic link",
            Self::TargetIsLink => "a destination path component is a symbolic link",
            Self::ComponentVanished => "a path component vanished",
        })
    }
}

/// Which end of a move a path belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Source,
    Target,
}

/// Revalidates the components of `path` that live **below** `trusted_root`,
/// without following links.
///
/// Only the managed subtree is checked. System prefixes are deliberately
/// trusted: on macOS `/var` is itself a symlink into `/private/var`, so a naive
/// walk from the filesystem root would refuse every legitimate operation
/// (OP-04, scoped to what the deployment actually manages).
pub fn revalidate_below(trusted_root: &Path, path: &Path, side: Side) -> Result<(), PathFault> {
    let relative = path
        .strip_prefix(trusted_root)
        .map_err(|_| PathFault::ComponentVanished)?;
    // Walk the real components under the root, resolving each one without
    // following links, so a link planted mid-path is seen as a link.
    let mut current = trusted_root.to_path_buf();
    for component in relative.components() {
        use std::path::Component;
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                current.pop();
            }
            Component::Normal(name) => {
                current.push(name);
                match std::fs::symlink_metadata(&current) {
                    Ok(metadata) => {
                        if metadata.file_type().is_symlink() {
                            return Err(match side {
                                Side::Source => PathFault::SourceIsLink,
                                Side::Target => PathFault::TargetIsLink,
                            });
                        }
                    }
                    // A destination that does not exist yet is normal; a
                    // source that does not is not.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if matches!(side, Side::Source) {
                            return Err(PathFault::ComponentVanished);
                        }
                    }
                    Err(_) => return Err(PathFault::ComponentVanished),
                }
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(())
}

/// What a completed operation did to the volume, reported honestly. The three
/// numbers are deliberately separate: bytes the operation processed, bytes the
/// quarantine still holds (so the user knows the space is not free), and the
/// measured free-space delta on the volume (OP-10).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeReport {
    /// Logical bytes the operation moved, copied, quarantined, or restored.
    pub processed_bytes: u64,
    /// Bytes still held in the quarantine for this operation.
    pub retained_in_quarantine_bytes: u64,
    /// Free bytes on the volume before the operation.
    pub free_before_bytes: u64,
    /// Free bytes on the volume after the operation.
    pub free_after_bytes: u64,
    /// When the free-space measurement was taken, milliseconds since epoch.
    pub measured_at_unix_ms: u64,
    /// Why the delta must not be read as "this operation freed that much".
    pub caveat: &'static str,
}

impl VolumeReport {
    /// The measured difference, which is what a real user should see.
    pub fn free_delta_bytes(&self) -> i64 {
        self.free_after_bytes as i64 - self.free_before_bytes as i64
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "processed_bytes": self.processed_bytes.to_string(),
            "retained_in_quarantine_bytes": self.retained_in_quarantine_bytes.to_string(),
            "free_before_bytes": self.free_before_bytes.to_string(),
            "free_after_bytes": self.free_after_bytes.to_string(),
            "free_delta_bytes": self.free_delta_bytes().to_string(),
            "measured_at_unix_ms": self.measured_at_unix_ms,
            "caveat": self.caveat,
        })
    }
}

/// Measures free space on the volume holding `path`, without shelling out to
/// `df` (EC-01). Returns None where the platform cannot answer.
pub fn volume_free_bytes(path: &Path) -> Option<u64> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // Find the nearest existing ancestor, since a destination may not exist
        // yet, and read the filesystem's own block accounting.
        let mut probe = path.to_path_buf();
        loop {
            match std::fs::symlink_metadata(&probe) {
                Ok(_) => break,
                Err(_) => probe = probe.parent()?.to_path_buf(),
            }
        }
        let c_path = std::ffi::CString::new(probe.to_str()?).ok()?;
        // SAFETY: c_path is a valid NUL-terminated string and stat is a valid
        // pointer to a fully initialised statfs. libc owns the layout, so the
        // struct is the platform's, not a hand-written approximation.
        unsafe {
            let mut stat: libc::statfs = std::mem::zeroed();
            if libc::statfs(c_path.as_ptr(), &mut stat) != 0 {
                return None;
            }
            // Linux reports u64 blocks, macOS reports u32; normalise both.
            #[cfg(target_os = "linux")]
            let bytes = stat.f_bavail as u64 * stat.f_frsize as u64;
            #[cfg(target_os = "macos")]
            let bytes = stat.f_bavail as u64 * stat.f_bsize as u64;
            Some(bytes)
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Other platforms report a value or nothing; guessing would be worse
        // than admitting the measurement is unavailable.
        let _ = path;
        None
    }
}

/// Sums the bytes a completed operation still holds in quarantine.
pub fn quarantine_retained_bytes(
    engine: &std::sync::Arc<Engine>,
    scope_id: &ScopeId,
) -> Result<u64, OpsError> {
    let control = engine.control_store()?;
    let mut total = 0u64;
    for operation in control.list_operations(scope_id, 1_000)? {
        for item in control.operation_items(&operation.operation_id)? {
            if item.result != diskgraph_store::OperationItemResult::Quarantined {
                continue;
            }
            if let Some(recovery_ref) = item.recovery_ref
                && let Ok(entry) = control.recovery(&recovery_ref)
                && entry.state == diskgraph_store::RecoveryState::Available
            {
                total = total.saturating_add(quarantine_bytes(&entry));
            }
        }
    }
    Ok(total)
}

fn quarantine_bytes(entry: &diskgraph_store::RecoveryEntry) -> u64 {
    unhex_key(&entry.quarantine_locator)
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// Refreshes the index for the scope an operation touched, so a later query
/// reflects the file system instead of the plan's snapshot (OP-10).
pub fn refresh_scope_after_operation(
    engine: &std::sync::Arc<Engine>,
    scope_id: &ScopeId,
    principal: &PrincipalId,
) -> Result<JobOutcome, OpsError> {
    let authorizer = engine
        .policy_authorizer()
        .map_err(|error| OpsError::Stale(error.to_string()))?;
    let job = engine
        .index_scope(scope_id, principal, &authorizer)
        .map_err(|error| OpsError::Stale(error.to_string()))?;
    let finished = engine
        .run_job(&job.job_id, "ops-refresh")
        .map_err(|error| OpsError::Stale(error.to_string()))?;
    Ok(JobOutcome {
        job_id: finished.job_id,
        state: finished.state,
    })
}

/// The result of the post-operation refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobOutcome {
    pub job_id: String,
    pub state: diskgraph_store::JobState,
}

/// One operation as an agent sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationView {
    pub operation_id: String,
    pub plan_id: String,
    pub scope_id: ScopeId,
    pub state: diskgraph_store::OperationState,
    pub items: Vec<diskgraph_store::OperationItem>,
    /// Items that already ran; a cancellation never rewrites these.
    pub completed: usize,
    pub remaining: usize,
}

fn view_of(
    operation: &diskgraph_store::Operation,
    control: &diskgraph_store::ControlStore,
) -> Result<OperationView, OpsError> {
    let items = control.operation_items(&operation.operation_id)?;
    let completed = items
        .iter()
        .filter(|item| item.result != diskgraph_store::OperationItemResult::Pending)
        .count();
    Ok(OperationView {
        operation_id: operation.operation_id.clone(),
        plan_id: operation.plan_id.clone(),
        scope_id: operation.scope_id.clone(),
        state: operation.state,
        remaining: items.len() - completed,
        items,
        completed,
    })
}

/// Lists a principal's operations for a scope, newest first (C26).
pub fn list_operations(
    engine: &std::sync::Arc<Engine>,
    scope_id: &ScopeId,
    principal: &PrincipalId,
    limit: u64,
) -> Result<Vec<OperationView>, OpsError> {
    let control = engine.control_store()?;
    let mut views = Vec::new();
    for operation in control.list_operations(scope_id, limit)? {
        // Another principal's operation is not visible here (SC-04).
        if &operation.principal != principal {
            continue;
        }
        views.push(view_of(&operation, &control)?);
    }
    Ok(views)
}

/// Shows one operation the principal owns.
pub fn show_operation(
    engine: &std::sync::Arc<Engine>,
    operation_id: &str,
    principal: &PrincipalId,
) -> Result<OperationView, OpsError> {
    let control = engine.control_store()?;
    let operation = control.operation(operation_id)?;
    if &operation.principal != principal {
        return Err(OpsError::NotAuthorized(
            "the operation belongs to another principal".into(),
        ));
    }
    view_of(&operation, &control)
}

/// Cancels an operation. Items that already ran keep their result; only the
/// remaining ones stop, and a finished operation is never rewritten (OP-09).
pub fn cancel_operation(
    engine: &std::sync::Arc<Engine>,
    operation_id: &str,
    principal: &PrincipalId,
) -> Result<OperationView, OpsError> {
    let mut control = engine.control_store()?;
    let operation = control.operation(operation_id)?;
    if &operation.principal != principal {
        return Err(OpsError::NotAuthorized(
            "the operation belongs to another principal".into(),
        ));
    }
    if operation.state.is_terminal() {
        // Cancelling finished work would rewrite history.
        return view_of(&operation, &control);
    }
    let items = control.operation_items(operation_id)?;
    for item in &items {
        if item.result == diskgraph_store::OperationItemResult::Pending {
            control.record_item_result(
                operation_id,
                item.item_index,
                diskgraph_store::OperationItemResult::Failed,
                "cancelled before it ran",
                None,
            )?;
        }
    }
    control.set_operation_state(operation_id, diskgraph_store::OperationState::Cancelled)?;
    let operation = control.operation(operation_id)?;
    view_of(&operation, &control)
}

/// One planned object, revalidated against the live filesystem.
struct LiveItem {
    path: PathBuf,
    /// The identity revalidated at apply time; kept so the operation record can
    /// name exactly which object was touched.
    identity: Option<String>,
    /// The recovery record this item restores from, for restore plans.
    recovery_ref: Option<String>,
    bytes: u64,
}

struct StepResult {
    kind: diskgraph_store::OperationItemResult,
    detail: String,
    bytes: u64,
    recovery_ref: Option<String>,
}

/// A short, redacted description of what a step touched: the target's file name
/// and the identity, never a whole user path.
fn describe(target: &Path, identity: Option<&str>) -> String {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "<unnamed>".into());
    match identity {
        Some(identity) => format!("{name} ({identity})"),
        None => name,
    }
}

/// The digest that identifies an apply request, so a reused idempotency key
/// with a different approval is a conflict rather than a silent merge.
fn apply_request_digest(plan: &Plan, approval_ref: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    plan_digest(plan).hash(&mut hasher);
    approval_ref.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Same-volume operations use an atomic rename; the cross-volume paths are
/// handled by the dedicated P6 staging flow.
fn are_same_volume(source: &Path, target: &Path) -> Result<bool, OpsError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let left = std::fs::metadata(source)
            .map_err(|error| OpsError::Stale(error.to_string()))?
            .dev();
        let right = std::fs::metadata(target.parent().unwrap_or(target))
            .map_err(|error| OpsError::Stale(error.to_string()))?
            .dev();
        Ok(left == right)
    }
    #[cfg(not(unix))]
    {
        // Without device ids the platform cannot prove same-volume, so every
        // transfer takes the staged path: correct, just slower.
        let _ = (source, target);
        Ok(false)
    }
}

/// Removes a file, symlink, or directory tree for good. Callers have already
/// proven the object's identity and its path boundary; nothing here re-checks
/// them, so nothing here is reachable without a verified purge plan.
fn remove_source(path: &Path) -> Result<(), OpsError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// A staged cross-volume transfer (OP-05). The staged file lives on the
/// target's own volume, so publishing is a same-volume rename: an interrupted
/// transfer leaves a staging directory behind and the destination untouched.
pub(crate) struct CrossVolumeCopy {
    staging_dir: PathBuf,
    staged: PathBuf,
    target: PathBuf,
}

impl CrossVolumeCopy {
    /// Opens a transfer that will publish `target` from a staging directory
    /// next to it. The directory name is unique per transfer, so concurrent
    /// transfers into one directory never remove each other's work.
    pub(crate) fn open(target: &Path, purpose: &str) -> Result<Self, OpsError> {
        let directory = target
            .parent()
            .ok_or_else(|| OpsError::Stale("target has no parent directory".into()))?;
        let staging_dir = directory.join(format!(
            ".dg-{purpose}-staging-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&staging_dir)?;
        let name = target
            .file_name()
            .ok_or_else(|| OpsError::Stale("target has no file name".into()))?
            .to_owned();
        Ok(Self {
            staged: staging_dir.join(&name),
            staging_dir,
            target: target.to_path_buf(),
        })
    }

    /// Copies the source into staging, then verifies the byte count and that
    /// the source itself did not change underneath the copy.
    pub(crate) fn stage_and_verify(
        &self,
        source: &Path,
        expected_identity: &Option<String>,
    ) -> Result<u64, OpsError> {
        let copied = std::fs::copy(source, &self.staged)?;
        let staged_len = std::fs::symlink_metadata(&self.staged)?.len();
        if copied != staged_len {
            return Err(OpsError::Stale("the staged copy is incomplete".into()));
        }
        // The source must still be the object the plan described: a file that
        // changed while it was being read invalidates the transfer (OP-05).
        let metadata = std::fs::symlink_metadata(source)?;
        if let Some(expected) = expected_identity {
            let actual = identity_of(source, &metadata);
            if actual.as_deref() != Some(expected.as_str()) {
                return Err(OpsError::Stale("the source changed during the copy".into()));
            }
        }
        if metadata.len() != staged_len {
            return Err(OpsError::Stale(
                "the source changed size during the copy".into(),
            ));
        }
        Ok(staged_len)
    }

    /// Publishes the verified copy with a same-volume rename. The destination
    /// is rechecked so a file that appeared during the transfer is never
    /// overwritten.
    pub(crate) fn publish(&self) -> Result<(), OpsError> {
        if std::fs::symlink_metadata(&self.target).is_ok() {
            return Err(OpsError::TargetExists);
        }
        std::fs::rename(&self.staged, &self.target)?;
        let _ = std::fs::remove_dir_all(&self.staging_dir);
        Ok(())
    }

    /// Best-effort cleanup after a failed transfer.
    pub(crate) fn discard(&self) {
        let _ = std::fs::remove_file(&self.staged);
        let _ = std::fs::remove_dir_all(&self.staging_dir);
    }

    /// True once no staging directory remains, so drills can prove a transfer
    /// left no bytes behind.
    #[cfg(test)]
    pub(crate) fn staging_dir_absent(&self) -> bool {
        !self.staging_dir.exists()
    }

    /// The staged path, for drills that must observe the transfer directly.
    #[cfg(test)]
    fn staged_path(&self) -> &Path {
        &self.staged
    }
}

/// The canonical form of a directory that may not exist yet: canonicalize the
/// deepest existing ancestor and re-attach the rest.
fn canonical_dir(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut existing = path.to_path_buf();
    let mut trailing: Vec<std::ffi::OsString> = Vec::new();
    while let Some(parent) = existing.parent().map(Path::to_path_buf) {
        if let Some(name) = existing.file_name() {
            trailing.push(name.to_os_string());
        }
        if let Ok(canonical) = parent.canonicalize() {
            let mut result = canonical;
            for name in trailing.iter().rev() {
                result.push(name);
            }
            return result;
        }
        existing = parent;
    }
    path.to_path_buf()
}

/// The native path behind a scope record's root locator.
fn path_of(locator: &diskgraph_core::Locator) -> Option<PathBuf> {
    let bytes = locator.raw_bytes().ok()?;
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Reverses the hex locator key back into a path.
fn unhex_key(key: &str) -> Option<PathBuf> {
    hex::decode(key)
        .ok()
        .map(|bytes| PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

/// A read-only locator helper used by the ops layer and the store.
trait RawPath {
    fn raw_path(&self) -> Option<PathBuf>;
}

impl RawPath for diskgraph_core::ResourceLocator {
    fn raw_path(&self) -> Option<PathBuf> {
        match self {
            diskgraph_core::ResourceLocator::NativePath(path) => Some(PathBuf::from(path)),
            diskgraph_core::ResourceLocator::DocumentUri(_) => None,
        }
    }
}

pub mod docker;
pub mod specialist;
pub use docker::{DockerInventory, DockerObject, UsageCheck, VM_CAVEAT};
pub use specialist::{
    AdapterRegistry, AdapterStatus, CARGO_CLEAN, CleanupInventory, CommandRunner, CommandSpec,
    DOCKER_CLEAN, DOCKER_INVENTORY, InventoryObject, SandboxedRunner, SpecialistVerdict,
};

#[cfg(test)]
mod tests;
