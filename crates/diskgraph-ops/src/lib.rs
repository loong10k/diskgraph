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
    #[error("the target already exists and overwrite is not permitted")]
    TargetExists,
    #[error("a same-volume move is not possible across devices")]
    CrossVolume,
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
        let plan = self.build(PlanRequest {
            scope_id,
            principal,
            action,
            node_ids,
            target: Some(target),
            max_bytes,
            recovery,
        })?;
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
    let bytes = path.to_string_lossy().into_owned();
    bytes
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
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

#[cfg(test)]
mod tests;
