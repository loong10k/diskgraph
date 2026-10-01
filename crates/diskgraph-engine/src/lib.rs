//! Engine orchestration (P1 tasks 2.4 / 2.8 / 2.9 / 2.12, specs ST-01 /
//! RT-01 / RT-02 / RT-04): authorized scope services, durable scan jobs with
//! owner fencing and cooperative cancellation, staging-based atomic
//! publication, and per-scan budgets. The engine never mutates user files.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diskgraph_core::{
    Authorizer, BudgetDecision, BudgetUsage, BusinessError, CapacityReading, DiskGraph, Grant,
    Locator, Permission, PolicyAuthorizer, PrincipalId, ResourceLocator, ResourceRef, ScanBudget,
    ScanBudgetStop, ScanExclusions, ScanWindow, ScopeId, ServerId, StorageArea, Watermark,
};
use diskgraph_store::{
    ControlStore, JobKind, JobRecord, JobState, ScopeRecord, SqliteSnapshotStore, StoreError,
};

mod collectors;
pub mod content;
pub mod live_evidence;
mod queries;
mod runner;
pub mod verify;

pub use collectors::{
    COLLECTOR_ID, COLLECTOR_VERSION, ProjectBatch, RULE_VERSION, collect_projects,
};
pub use queries::{
    ExploreSummary, ImpactEntry, Propagation, cursor_context, explore, impact, impact_propagation,
    incompatibility_name, search_nodes,
};
pub use runner::JobRunner;

/// The result of comparing two revisions: what was compared, and what differs.
///
/// Carries both roots and both node counts so a caller can tell a comparison
/// of two checkouts from a comparison of two million-file trees, and never
/// has to assume the second.
pub struct ComparisonReport {
    pub left_revision: String,
    pub right_revision: String,
    pub left_root: diskgraph_core::ResourceLocator,
    pub right_root: diskgraph_core::ResourceLocator,
    pub left_nodes: usize,
    pub right_nodes: usize,
    pub rows: Vec<CompareRow>,
    pub summary: diskgraph_core::Summary,
}

/// One path's comparison, owned so a report outlives the graphs it came from.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CompareRow {
    pub path: String,
    pub verdict: diskgraph_core::Verdict,
    pub left_bytes: Option<u64>,
    pub right_bytes: Option<u64>,
}

impl ComparisonReport {
    /// The shape a caller prints, in the order the fields matter.
    pub fn to_json(&self, limit: Option<usize>) -> serde_json::Value {
        let shown: Vec<serde_json::Value> = self
            .rows
            .iter()
            .take(limit.unwrap_or(self.rows.len()))
            .map(|row| {
                serde_json::json!({
                    "path": row.path,
                    "verdict": row.verdict,
                    "left_bytes": row.left_bytes,
                    "right_bytes": row.right_bytes,
                })
            })
            .collect();
        serde_json::json!({
            "left": {
                "revision_id": self.left_revision,
                "root": self.left_root,
                "nodes": self.left_nodes,
            },
            "right": {
                "revision_id": self.right_revision,
                "root": self.right_root,
                "nodes": self.right_nodes,
            },
            "summary": self.summary,
            "actionable": self.summary.actionable(),
            "entries": shown.len(),
            "rows": shown,
        })
    }
}

/// The filesystem path a root node was indexed from.
fn native_path(root: &diskgraph_core::DiskNode) -> Result<String, EngineError> {
    match &root.locator {
        diskgraph_core::ResourceLocator::NativePath(path) => Ok(path.clone()),
        diskgraph_core::ResourceLocator::DocumentUri(_) => Err(EngineError::Business(
            diskgraph_core::BusinessError::InvalidArgument,
        )),
    }
}

/// The size change of one path between two published revisions, owned/// rather than borrowed: the answer costs two rows, so there is no reason to
/// hold a whole graph alive to describe them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionGrowth {
    pub before: diskgraph_core::DiskNode,
    pub after: diskgraph_core::DiskNode,
    pub delta_bytes: i128,
}

/// Engine construction options; the data directory holds both databases.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub data_dir: PathBuf,
    /// Hard node budget per scan (RT-04). Exceeding it fails the job and
    /// never publishes a partial latest revision.
    pub max_nodes_per_scan: u64,
    /// Maximum active (queued or running) jobs one principal may hold
    /// (P4 task 5.5, spec MCP-06). Excess requests are refused with
    /// `resource_exhausted` instead of queueing without bound.
    pub max_active_jobs_per_principal: u32,
    /// The walk budget: nodes, duration, staging bytes, and write batch size
    /// (P1 task 2.9, spec RT-02). Reaching a hard limit stops the walk for a
    /// named reason instead of returning less data without saying so.
    pub scan_budget: ScanBudget,
    /// Where the engine refuses new work once the data directory fills up
    /// (P1 task 2.12, spec RT-04). A refusal never deletes anything.
    pub capacity_watermark: Watermark,
    /// How the walk behaves, using disktree's own option contract: a scope
    /// indexed with one set of options is only ever comparable with a scope
    /// indexed the same way (the snapshot records these verbatim).
    pub scan_options: diskgraph_disktree_core::scan::ScanOptions,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("diskgraph-data"),
            max_nodes_per_scan: 2_000_000,
            max_active_jobs_per_principal: 8,
            scan_budget: ScanBudget {
                max_nodes: 2_000_000,
                ..ScanBudget::default()
            },
            capacity_watermark: Watermark {
                warn_above_bytes: 8 << 30,
                refuse_above_bytes: 16 << 30,
            },
            scan_options: diskgraph_disktree_core::scan::ScanOptions::default(),
        }
    }
}

/// Engine failures: storage, IO, or a business error carrying its stable code.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("engine poisoned by a previous panic")]
    Poisoned,
    #[error("{0}")]
    Business(#[from] BusinessError),
}

/// An engine error plus which catalog entry produced it, so MCP and the CLI
/// can name the family without parsing message text.
#[derive(Debug)]
pub struct ContextualEngineError {
    pub error: EngineError,
    pub context: String,
}

impl std::fmt::Display for ContextualEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (while serving {})", self.error, self.context)
    }
}

impl std::error::Error for ContextualEngineError {}

/// The scope that authorizes server administration (scope add/remove, serve).
pub fn admin_scope() -> ScopeId {
    ScopeId::new("diskgraph-admin").expect("constant is valid")
}

/// What one explanation returns: the entity, its edges, and their evidence.
pub type Explanation = (
    diskgraph_core::Entity,
    Vec<diskgraph_core::RelationEdge>,
    Vec<diskgraph_core::EvidenceRecord>,
);

/// The shared DiskGraph service: one data directory, two databases, durable jobs.
pub struct Engine {
    data_dir: PathBuf,
    max_nodes_per_scan: u64,
    scan_budget: ScanBudget,
    capacity_watermark: Watermark,
    max_active_jobs_per_principal: u32,
    scan_options: diskgraph_disktree_core::scan::ScanOptions,
    graph: Mutex<SqliteSnapshotStore>,
    control: Mutex<ControlStore>,
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl EngineError {
    /// Attaches the catalog entry that produced this error.
    pub fn with_context(self, context: impl Into<String>) -> ContextualEngineError {
        ContextualEngineError {
            error: self,
            context: context.into(),
        }
    }
}

impl From<ContextualEngineError> for EngineError {
    fn from(contextual: ContextualEngineError) -> Self {
        contextual.error
    }
}

impl Engine {
    /// Opens (creating if needed) the engine's data directory with both stores.
    pub fn open(config: EngineConfig) -> Result<Self, EngineError> {
        std::fs::create_dir_all(&config.data_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&config.data_dir)?.permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&config.data_dir, permissions)?;
        }
        let graph = SqliteSnapshotStore::open(&config.data_dir.join("diskgraph.sqlite"))?;
        let control = ControlStore::open(&config.data_dir.join("diskgraph-control.sqlite"))?;
        Ok(Self {
            data_dir: config.data_dir,
            max_nodes_per_scan: config.max_nodes_per_scan,
            scan_budget: config.scan_budget,
            capacity_watermark: config.capacity_watermark,
            max_active_jobs_per_principal: config.max_active_jobs_per_principal,
            scan_options: config.scan_options.clone(),
            graph: Mutex::new(graph),
            control: Mutex::new(control),
            cancellations: Mutex::new(HashMap::new()),
        })
    }

    /// Where both databases live (diagnostics only).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Mints once and then serves the persistent server identity (ST-05).
    pub fn server_id(&self) -> Result<ServerId, EngineError> {
        Ok(self.control()?.ensure_server()?)
    }

    /// Registers or idempotently returns a scope for a native root (SC-01).
    pub fn register_scope(
        &self,
        root: &Path,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        self.require(
            authorizer,
            principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        )?;
        let canonical = root.canonicalize()?;
        let locator = Locator::from_native_path(&canonical);
        let volume_id = locator_volume_id(&locator);
        let mut control = self.control()?;
        let scope_id = control.register_scope(&locator, volume_id.as_deref())?;
        // The registrar receives scope-local index/metadata/operation rights;
        // server administration itself stays bound to the admin scope.
        let version = control.policy_version()?;
        if version > 0 {
            for permission in [
                Permission::IndexWrite,
                Permission::MetadataRead,
                Permission::OperationView,
            ] {
                control.upsert_grant(&Grant {
                    principal: principal.clone(),
                    permission,
                    scope: scope_id.clone(),
                    policy_version: version,
                })?;
            }
        }
        Ok(scope_id)
    }

    /// Lists registered scopes the principal may read metadata for. Listing is
    /// a metadata read scoped to each entry: the registry is visible exactly
    /// as far as the caller's grants reach, and an ungranted principal sees
    /// nothing at all.
    pub fn list_scopes(
        &self,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<ScopeRecord>, EngineError> {
        let authorizer_is_permissive = matches!(
            authorizer.decide(principal, &Permission::MetadataRead, &admin_scope()),
            diskgraph_core::Decision::Allowed
        );
        let scopes = self.control()?.list_scopes()?;
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let allowed = scopes
            .into_iter()
            .filter(|scope| {
                matches!(
                    authorizer.decide(principal, &Permission::MetadataRead, &scope.scope_id),
                    diskgraph_core::Decision::Allowed
                )
            })
            .collect::<Vec<_>>();
        // A principal whose only grant is administration sees the whole
        // registry; every other principal sees only its own scopes.
        if allowed.is_empty() && authorizer_is_permissive {
            return Ok(self.control()?.list_scopes()?);
        }
        Ok(allowed)
    }

    /// Revokes a scope; lookups and new jobs stop (SC-04). Files, snapshots,
    /// and control history are never deleted.
    pub fn revoke_scope(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.require(
            authorizer,
            principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        )?;
        self.control()?.revoke_scope(scope_id)?;
        Ok(())
    }

    /// Loads one scope.
    pub fn scope(&self, scope_id: &ScopeId) -> Result<ScopeRecord, EngineError> {
        Ok(self.control()?.scope(scope_id)?)
    }

    /// The live policy authorizer rebuilt from the control store.
    pub fn policy_authorizer(&self) -> Result<PolicyAuthorizer, EngineError> {
        Ok(self.control()?.authorizer()?)
    }

    /// One-time local bootstrap (single-user CLI/service mode): publishes
    /// policy v1 if absent and grants this principal server administration,
    /// index management, metadata reads, and operation views on the admin
    /// scope. Authorization for everything else stays default-deny.
    pub fn bootstrap_local_admin(&self, principal: &PrincipalId) -> Result<(), EngineError> {
        let mut control = self.control()?;
        // Publish the initial version only when nothing was ever published:
        // a revoked policy must stay revoked until an explicit republish.
        if control.policy_state()?.is_none() {
            control.publish_policy_version(1)?;
        }
        let version = control.policy_version()?;
        for permission in [
            Permission::ScopeAdmin,
            Permission::IndexWrite,
            Permission::MetadataRead,
            Permission::OperationView,
        ] {
            control.upsert_grant(&Grant {
                principal: principal.clone(),
                permission,
                scope: admin_scope(),
                policy_version: version,
            })?;
        }
        // Renew every existing grant into the current epoch, so a policy bump
        // does not silently strip scope-local rights the administrator already
        // issued; explicit revocation is what takes rights away (SC-04). A
        // revoked policy is left untouched: renewal must not resurrect it.
        if let Some((_, true)) = control.policy_state()? {
            return Ok(());
        }
        for grant in control.all_grants()? {
            control.upsert_grant(&Grant {
                policy_version: version,
                ..grant
            })?;
        }
        Ok(())
    }

    /// Grants or withdraws the right to read file contents inside one scope.
    ///
    /// Registering a scope hands out index, metadata and view rights and
    /// nothing more, because reading a file's bytes is a different question
    /// from reading its size (D13). A caller that wants to verify a
    /// comparison against contents asks for this first, and can take it back;
    /// a grant nobody asked for is exactly the kind that outlives its reason.
    pub fn set_content_read(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        allow: bool,
    ) -> Result<(), EngineError> {
        let mut control = self.control_store()?;
        let version = control.policy_version()?;
        if version == 0 {
            // No policy epoch means no grants exist to add one to.
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if allow {
            control.upsert_grant(&Grant {
                principal: principal.clone(),
                permission: Permission::ContentRead,
                scope: scope_id.clone(),
                policy_version: version,
            })?;
        } else {
            control.revoke_grant(principal, &Permission::ContentRead, scope_id)?;
        }
        Ok(())
    }

    /// Creates (or merges into) a durable index job (C02, AI-03).
    pub fn index_scope(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        self.create_scan_job(scope_id, JobKind::Index, principal, authorizer)
    }

    /// Explicit controlled rescan of a registered scope (C03): merges into any
    /// active job for the scope and publishes a fresh snapshot + revision.
    pub fn sync_scope(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        self.create_scan_job(scope_id, JobKind::Sync, principal, authorizer)
    }

    fn create_scan_job(
        &self,
        scope_id: &ScopeId,
        kind: JobKind,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<JobRecord, EngineError> {
        self.require(authorizer, principal, &Permission::IndexWrite, scope_id)?;
        let mut control = self.control()?;
        // A merge into an already-active job for this scope is free and never
        // consumes quota (AI-03); only genuinely new jobs are counted.
        if let Some(active) = control.active_job_for_scope(scope_id)? {
            return Ok(active);
        }
        // Per-principal job quota (P4 task 5.5): merged jobs count once
        // because the merge returns the existing record.
        if control.active_job_count_for_principal(principal)?
            >= u64::from(self.max_active_jobs_per_principal)
        {
            return Err(EngineError::Business(BusinessError::ResourceExhausted));
        }
        let job = control.create_job(scope_id, kind, principal)?;
        drop(control);
        self.cancellations()?.entry(job.job_id.clone()).or_default();
        Ok(job)
    }

    /// Lists snapshots for a scope's root (C05 list/show), newest first.
    pub fn list_snapshots(
        &self,
        scope_id: &ScopeId,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<diskgraph_core::DiskSnapshot>, EngineError> {
        self.require(authorizer, principal, &Permission::MetadataRead, scope_id)?;
        let root = ResourceLocator::NativePath(self.scope(scope_id)?.root.display().to_owned());
        Ok(self.graph()?.list_snapshots(Some(&root), limit, offset)?)
    }

    /// Pins or unpins one snapshot (C05 pin, retention protection).
    pub fn pin_snapshot(
        &self,
        snapshot_id: &str,
        pinned: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.require_write_for_snapshot(snapshot_id, principal, authorizer)?;
        self.graph()?.pin_snapshot(snapshot_id, pinned)?;
        Ok(())
    }

    /// Removes one snapshot from graph history (C05 remove). Refused when
    /// pinned or referenced by published revisions; never touches user files.
    pub fn remove_snapshot(
        &self,
        snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.require_write_for_snapshot(snapshot_id, principal, authorizer)?;
        self.graph()?.remove_snapshot(snapshot_id)?;
        Ok(())
    }

    /// Snapshot management carries index:write (C05); any scope the principal
    /// can write is sufficient for retention edits on the shared graph index.
    fn require_write_for_snapshot(
        &self,
        _snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        // P1: retention edits authorize against the admin scope because the
        // graph index is shared across scopes; scope-scoped retention arrives
        // with per-scope graph namespaces (ST-04 full semantics, P2).
        self.require(
            authorizer,
            principal,
            &Permission::IndexWrite,
            &admin_scope(),
        )
    }

    /// Cancels a job (C26 semantics arrive in P5; P1 cancels scans).
    /// Queued jobs are cancelled immediately; running jobs observe the flag
    /// between walk batches and never advance `latest`.
    pub fn cancel_job(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        let job = self.control()?.job(job_id)?;
        self.require(
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        if let Some(flag) = self.cancellations()?.get(job_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let mut control = self.control()?;
        match control.cancel_queued(job_id) {
            Ok(true) => Ok(()),
            Ok(false) => Ok(()), // running: the runner will observe the flag
            Err(error) => Err(error.into()),
        }
    }

    /// Claims and runs one job to a terminal state, returning the durable
    /// record (RT-01: reconnection queries this instead of the connection).
    pub fn run_job(&self, job_id: &str, owner: &str) -> Result<JobRecord, EngineError> {
        {
            let mut control = self.control()?;
            control.claim_job(job_id, owner)?;
        }
        let cancel = {
            let mut cancellations = self.cancellations()?;
            Arc::clone(cancellations.entry(job_id.to_owned()).or_default())
        };
        let outcome = self.execute_scan(job_id, owner, &cancel);
        let final_state = if outcome.is_ok() {
            JobState::Completed
        } else {
            JobState::Failed
        };
        let record = self.control()?.finish_job(job_id, owner, final_state)?;
        self.cancellations()?.remove(job_id);
        outcome?;
        Ok(record)
    }

    /// Loads one durable job record (reconnect-safe business state, MCP-05 seed).
    pub fn job_status(&self, job_id: &str) -> Result<JobRecord, EngineError> {
        Ok(self.control()?.job(job_id)?)
    }

    /// Publishes a new policy version; grants from older versions stop
    /// applying and every cursor issued under them is refused (SC-04, P4-5.9).
    pub fn publish_policy_version(
        &self,
        version: u64,
        principal: &PrincipalId,
        _authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        // Management validates against the durable admin grant, not the live
        // authorizer: a revoked policy must remain republishable (SC-04).
        if !self.control()?.holds_admin(principal)? {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let mut control = self.control()?;
        control.publish_policy_version(version)?;
        // Existing grants carry into the new epoch unless explicitly revoked:
        // a version bump expires forged/stale artifacts (cursors), not the
        // administrator's standing grants (SC-04).
        for grant in control.all_grants()? {
            control.upsert_grant(&Grant {
                policy_version: version,
                ..grant
            })?;
        }
        Ok(())
    }

    /// Revokes the whole policy; nothing is authorized until republished.
    pub fn revoke_policy(
        &self,
        principal: &PrincipalId,
        _authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        if !self.control()?.holds_admin(principal)? {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        self.control()?.revoke_policy()?;
        Ok(())
    }

    /// Every queued job across scopes; the job runner drains this list.
    pub fn queued_jobs(&self) -> Result<Vec<JobRecord>, EngineError> {
        Ok(self.control()?.list_queued_jobs()?)
    }

    /// The latest published revision for a scope, if it has ever published.
    pub fn latest_revision(&self, scope_id: &ScopeId) -> Result<Option<String>, EngineError> {
        let scope = self.control()?.scope(scope_id)?;
        let root = ResourceLocator::NativePath(scope.root.display().to_owned());
        Ok(self.graph()?.latest_revision_for_root(&root)?)
    }

    /// Loads the v1 graph behind one published revision.
    pub fn load_revision(&self, revision_id: &str) -> Result<DiskGraph, EngineError> {
        Ok(self.graph()?.load_revision(revision_id)?)
    }

    /// What a sync between two revisions would do, as a plan and nothing
    /// else.
    ///
    /// Reads both sides the way `compare_revisions` does, because a plan is
    /// built from the comparison and not from a second, cheaper walk: a plan
    /// that did not see every path would be a plan of what it happened to
    /// look at. It writes nothing, and there is no argument that would make
    /// it.
    pub fn sync_plan(
        &self,
        left_revision: &str,
        right_revision: &str,
        method: diskgraph_core::SyncMethod,
        tolerance_seconds: i64,
    ) -> Result<diskgraph_core::SyncPlan, EngineError> {
        let left = self.load_revision(left_revision)?;
        let right = self.load_revision(right_revision)?;
        let (rows, _summary) = diskgraph_core::compare::compare(
            left.root(),
            &left.nodes,
            right.root(),
            &right.nodes,
            tolerance_seconds,
        );
        Ok(diskgraph_core::build_sync_plan(
            method,
            &rows,
            &native_path(left.root())?,
            &native_path(right.root())?,
        ))
    }

    /// Compares two published revisions, whatever their roots are.
    ///
    /// This is the one query that still loads both sides whole, because a
    /// comparison has to see every path on both to know which are missing.
    /// The report carries both node counts so a caller can see what that cost
    /// before repeating it: a diff that walks both sides level by level is the
    /// obvious next step, and until it exists, a four-million-node comparison
    /// is a large request rather than a cheap one.
    pub fn compare_revisions(
        &self,
        left_revision: &str,
        right_revision: &str,
        tolerance_seconds: i64,
    ) -> Result<ComparisonReport, EngineError> {
        let left = self.load_revision(left_revision)?;
        let right = self.load_revision(right_revision)?;
        // The core comparison borrows the nodes it walks, and the two graphs
        // are local here, so what leaves this function owns only what a
        // caller needs to render a row: the path, the verdict, and the two
        // sizes. A caller that wants whole nodes re-reads them by path.
        let (compared, summary) = diskgraph_core::compare::compare(
            left.root(),
            &left.nodes,
            right.root(),
            &right.nodes,
            tolerance_seconds,
        );
        let rows = compared
            .into_iter()
            .map(|row| CompareRow {
                path: row.path,
                verdict: row.verdict,
                left_bytes: row.left.map(|node| node.subtree_bytes),
                right_bytes: row.right.map(|node| node.subtree_bytes),
            })
            .collect();
        Ok(ComparisonReport {
            left_revision: left_revision.to_owned(),
            right_revision: right_revision.to_owned(),
            left_root: left.root().locator.clone(),
            right_root: right.root().locator.clone(),
            left_nodes: left.nodes.len(),
            right_nodes: right.nodes.len(),
            rows,
            summary,
        })
    }

    /// Explain one entity of a published revision: the entity, its edges, and
    /// their evidence records (C14, EV-02). Unknown entities stay unknown.
    pub fn explain_entity(
        &self,
        revision_id: &str,
        entity_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Option<Explanation>, EngineError> {
        self.require_read_for_revision(revision_id, principal, authorizer)?;
        let graph = self.graph()?;
        let revision = graph.revision(revision_id)?;
        let Some(entity) = graph.entity(&revision.snapshot_id, entity_id)? else {
            return Ok(None);
        };
        let mut edges = graph.edges_from(&revision.snapshot_id, entity_id, None)?;
        edges.extend(graph.edges_to(&revision.snapshot_id, entity_id, None)?);
        let edge_ids: Vec<String> = edges.iter().map(|edge| edge.edge_id.clone()).collect();
        let evidence = graph.evidence_for_edges(&revision.snapshot_id, &edge_ids)?;
        Ok(Some((entity, edges, evidence)))
    }

    /// Typed relations of one entity with direction and optional filter (C13).
    pub fn related(
        &self,
        revision_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
        outgoing: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<diskgraph_core::RelationEdge>, EngineError> {
        self.require_read_for_revision(revision_id, principal, authorizer)?;
        let graph = self.graph()?;
        let revision = graph.revision(revision_id)?;
        if outgoing {
            Ok(graph.edges_from(&revision.snapshot_id, entity_id, relation)?)
        } else {
            Ok(graph.edges_to(&revision.snapshot_id, entity_id, relation)?)
        }
    }

    /// Every typed edge of one published revision, for relation-shaped
    /// traversals (impact and friends).
    pub fn all_edges(
        &self,
        revision_id: &str,
    ) -> Result<Vec<diskgraph_core::RelationEdge>, EngineError> {
        let graph = self.graph()?;
        let revision = graph.revision(revision_id)?;
        Ok(graph.all_edges(&revision.snapshot_id)?)
    }

    fn require_read_for_revision(
        &self,
        _revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        // P2: relations live in the shared graph index; reads authorize
        // against metadata:read on the admin scope until per-scope graph
        // namespaces arrive (ST-04 full semantics, tracked for P2 wrap-up).
        self.require(
            authorizer,
            principal,
            &Permission::MetadataRead,
            &admin_scope(),
        )
    }

    /// The root node of a published revision - the entry a du-style summary
    /// reads its total from. One row, no graph materialization.
    pub fn revision_root_node(
        &self,
        revision_id: &str,
    ) -> Result<diskgraph_core::DiskNode, EngineError> {
        let graph = self.graph()?;
        let record = graph.revision(revision_id)?;
        graph
            .root_node(&record.snapshot_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))
    }

    /// One node in a published revision, addressed by its path under the
    /// root.
    ///
    /// Walks the path one level at a time, so the cost is one indexed lookup
    /// per segment rather than a scan. Loading the revision to answer "how
    /// did this directory change" was the only way before, and on a
    /// four-million-node index that meant materializing four million nodes to
    /// read two of them.
    pub fn revision_node_at(
        &self,
        revision_id: &str,
        relative: &std::path::Path,
    ) -> Result<Option<diskgraph_core::DiskNode>, EngineError> {
        use diskgraph_core::ResourceLocator;
        let graph = self.graph()?;
        let record = graph.revision(revision_id)?;
        let mut current = match graph.root_node(&record.snapshot_id)? {
            Some(root) => root,
            None => return Ok(None),
        };
        for segment in relative.components() {
            let name = segment.as_os_str().to_string_lossy().into_owned();
            if name == "." || name == ".." {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            current = match graph.child_named(&record.snapshot_id, current.id, &name)? {
                Some(child) => child,
                None => return Ok(None),
            };
        }
        // A path that names the root itself is the root, not a miss.
        debug_assert!(matches!(current.locator, ResourceLocator::NativePath(_)));
        Ok(Some(current))
    }

    /// How much one path grew between two published revisions.
    ///
    /// Reads the two nodes it needs and nothing else: the previous answer had
    /// to materialize both revisions, which on a four-million-node index meant
    /// four million nodes in memory to compare two of them. The comparability
    /// rules are the ones the in-memory version applies — a differing root,
    /// volume, or scan setting, an unknown volume, an earlier "after", or an
    /// incomplete scan on either side all answer "not comparable" rather than
    /// a number, because a size delta across incomparable scans is a fiction.
    pub fn growth_between(
        &self,
        before_revision: &str,
        after_revision: &str,
        relative: &Path,
    ) -> Result<Option<RevisionGrowth>, EngineError> {
        // The comparability check reads only metadata, so the store lock is
        // released before the node lookups: those take the same lock, and
        // holding it across them would wait on this thread's own guard.
        {
            let graph = self.graph()?;
            let before_record = graph.revision(before_revision)?;
            let after_record = graph.revision(after_revision)?;
            let before_snapshot = graph.snapshot(&before_record.snapshot_id)?;
            let after_snapshot = graph.snapshot(&after_record.snapshot_id)?;
            if before_snapshot.root != after_snapshot.root
                || before_snapshot.volume_id != after_snapshot.volume_id
                || after_snapshot.volume_id.is_none()
                || before_snapshot.settings != after_snapshot.settings
                || before_snapshot.captured_at_unix_ms > after_snapshot.captured_at_unix_ms
                || !before_snapshot.coverage.complete
                || !after_snapshot.coverage.complete
            {
                return Ok(None);
            }
        }
        let (Some(before), Some(after)) = (
            self.revision_node_at(before_revision, relative)?,
            self.revision_node_at(after_revision, relative)?,
        ) else {
            // A path absent from one side is a removal or an addition, never
            // a growth. Renames are not inferred, same as before.
            return Ok(None);
        };
        let delta_bytes = i128::from(after.subtree_bytes) - i128::from(before.subtree_bytes);
        Ok(Some(RevisionGrowth {
            before,
            after,
            delta_bytes,
        }))
    }

    /// One directory level of a published revision: the node itself and its
    /// children, ordered by observed size. The interactive surface loads a
    /// level at a time, so a multi-million-node index opens without
    /// materializing the whole graph.
    pub fn revision_layer(
        &self,
        revision_id: &str,
        parent_id: u64,
        limit: usize,
    ) -> Result<(diskgraph_core::DiskNode, Vec<diskgraph_core::DiskNode>), EngineError> {
        let graph = self.graph()?;
        let record = graph.revision(revision_id)?;
        let node = graph
            .node(&record.snapshot_id, parent_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let children = graph.children(&record.snapshot_id, parent_id, 0, limit as u64)?;
        Ok((node, children))
    }

    /// A depth-bounded tree view of a published revision. Uses the store's
    /// narrow read path: no JSON payloads, no locators, no full DiskNode
    /// materialization. Pre-v4 snapshots (NULL structured columns) fall
    /// back to the full load so older history renders identically.
    pub fn tree_view(
        &self,
        scope_id: &ScopeId,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        depth: usize,
        min_bytes: u64,
    ) -> Result<diskgraph_core::TreeView, EngineError> {
        let record = self.scope(scope_id)?;
        if record.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        self.require(authorizer, principal, &Permission::MetadataRead, scope_id)?;
        let snapshot_id = {
            let graph = self.graph()?;
            graph.revision(revision_id)?.snapshot_id
        };
        let rows = {
            let graph = self.graph()?;
            graph.tree_rows(&snapshot_id)?
        };
        if rows.is_empty() {
            // Pre-v4 snapshot: take the full-load path so old history works.
            let graph = self.load_revision(revision_id)?;
            return diskgraph_core::render_tree(&graph, depth, min_bytes)
                .map_err(|_| EngineError::Business(BusinessError::NotFound));
        }
        let nodes: Vec<diskgraph_core::TreeNode<'_>> = rows
            .iter()
            .map(
                |(
                    id,
                    parent_id,
                    name,
                    kind,
                    subtree_bytes,
                    direct_bytes,
                    files,
                    directories,
                    read_error,
                    category,
                )| {
                    diskgraph_core::TreeNode {
                        id: *id,
                        parent_id: *parent_id,
                        name: name.as_str(),
                        kind: match kind.as_str() {
                            "directory" => diskgraph_core::NodeKind::Directory,
                            "file" => diskgraph_core::NodeKind::File,
                            "symlink" => diskgraph_core::NodeKind::Symlink,
                            _ => diskgraph_core::NodeKind::Other,
                        },
                        subtree_bytes: *subtree_bytes as u64,
                        direct_bytes: *direct_bytes as u64,
                        files: *files as u64,
                        directories: *directories as u64,
                        read_error: *read_error != 0,
                        category_hint: category.as_deref(),
                    }
                },
            )
            .collect();
        diskgraph_core::render_tree_rows(&nodes, depth, min_bytes)
            .map_err(|_| EngineError::Business(BusinessError::NotFound))
    }

    /// Full reference for one node inside a published revision (SC-02 shape).
    pub fn resource_ref(
        &self,
        scope_id: &ScopeId,
        revision_id: &str,
        node_id: u64,
    ) -> Result<ResourceRef, EngineError> {
        Ok(ResourceRef {
            server_id: self.server_id()?,
            scope_id: scope_id.clone(),
            revision_id: diskgraph_core::RevisionId::new(revision_id.to_owned())
                .map_err(|error| EngineError::Store(StoreError::InvalidGraph(error.to_string())))?,
            node_id,
        })
    }

    fn execute_scan(
        &self,
        job_id: &str,
        owner: &str,
        cancel: &AtomicBool,
    ) -> Result<(), EngineError> {
        let job = self.control()?.job(job_id)?;
        let scope = self.control()?.scope(&job.scope_id)?;
        let root = scope.root.to_native_path().map_err(|error| {
            EngineError::Store(StoreError::InvalidGraph(format!(
                "scope root is not addressable on this platform: {error}"
            )))
        })?;

        // 2.5: the walk is an observation over a window, recorded with the
        // options that produced it, so two snapshots are only comparable when
        // they were configured the same way.
        let started_at_unix_ms = now_ms();
        let options = self.scan_options.clone();
        let handle =
            diskgraph_disktree_core::scan::ScanHandle::spawn(root.clone(), options.clone());
        let tree = loop {
            if cancel.load(Ordering::SeqCst) {
                handle.cancel();
            }
            match handle.poll() {
                Some(result) => break result,
                None => thread::sleep(Duration::from_millis(20)),
            }
        };
        let tree = tree.map_err(|error| {
            if cancel.load(Ordering::SeqCst) {
                EngineError::Business(BusinessError::Conflict)
            } else {
                EngineError::Io(error)
            }
        })?;
        let scanned = diskgraph_disktree::convert_tree(&root, &tree, scan_settings(&options))?;
        let window = ScanWindow {
            started_at_unix_ms,
            finished_at_unix_ms: now_ms(),
            options_fingerprint: options_fingerprint(&options),
            scanner_version: env!("CARGO_PKG_VERSION").to_owned(),
        };

        // 2.9: charge every observed node against the walk budget, so a scan
        // that exceeds a limit stops for a named reason instead of quietly
        // returning less than it found.
        let mut usage = BudgetUsage {
            elapsed_ms: window.duration_ms(),
            ..BudgetUsage::default()
        };
        let mut exclusions = ScanExclusions::default();
        let mut budget_stop: Option<ScanBudgetStop> = None;
        let total_bytes: u64 = scanned.nodes.iter().map(|node| node.v1.subtree_bytes).sum();

        // RT-04: the configured node ceiling is a hard refusal.
        if scanned.nodes.len() as u64 > self.max_nodes_per_scan {
            let mut graph = self.graph()?;
            let _ = graph.clear_staging(job_id);
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }

        // 2.9: charge the walk against its budget. A reached limit stops the
        // walk for a named reason instead of returning less data silently, and
        // a cancellation observed here is reported, never swallowed. The byte
        // charge is the node's OWN bytes, never its subtree aggregate: every
        // ancestor would otherwise bill the same file again, so a deep tree
        // would multiply its real size by its depth and stop on phantom bytes.
        for node in &scanned.nodes {
            match self
                .scan_budget
                .charge_node(&mut usage, node.v1.direct_bytes)
            {
                BudgetDecision::Continue => {}
                BudgetDecision::Stop(stop) => {
                    budget_stop = Some(stop);
                    break;
                }
            }
            if let Some(decision) = ScanBudget::observe_cancel(cancel.load(Ordering::SeqCst))
                && decision.is_stop()
            {
                budget_stop = Some(ScanBudgetStop::Cancelled);
                break;
            }
        }
        if let Some(stop) = budget_stop {
            // Record why the walk ended before refusing, so the job log can
            // explain itself. The named reason goes to the operator's stderr
            // too: a failed job must be diagnosable without a debugger.
            exclusions
                .error_summary
                .push(format!("scan stopped: {stop:?}"));
            eprintln!(
                "diskgraph: scan stopped: {stop:?} after {} nodes / {} charged bytes / {} ms",
                usage.nodes, usage.staged_bytes, usage.elapsed_ms
            );
            let _ = total_bytes;
            let mut graph = self.graph()?;
            let _ = graph.clear_staging(job_id);
            return Err(EngineError::Business(match stop {
                ScanBudgetStop::Cancelled => BusinessError::Conflict,
                _ => BusinessError::BudgetExceeded,
            }));
        }

        // Stage in bounded batches, then publish snapshot + revision + latest
        // pointer in one transaction (ST-01).
        let mut graph = self.graph()?;
        for batch in scanned.nodes.chunks(512) {
            graph.append_staging_nodes(
                job_id,
                &batch.iter().map(|node| node.v1.clone()).collect::<Vec<_>>(),
            )?;
        }
        let v1_nodes: Vec<diskgraph_core::DiskNode> =
            scanned.nodes.iter().map(|node| node.v1.clone()).collect();
        let revision_id = format!("rev-{}", uuid::Uuid::new_v4());
        let published_at = now_ms();
        let observed_graph = DiskGraph {
            snapshot: scanned.snapshot.clone(),
            nodes: v1_nodes,
            evidence: scanned.evidence,
        };
        let result = graph.publish_revision(job_id, &observed_graph, &revision_id, published_at);
        if let Err(error) = result {
            let _ = graph.clear_staging(job_id);
            let _ = self.control()?.finish_job(job_id, owner, JobState::Failed);
            return Err(error.into());
        }

        // Deterministic collectors run against the just-published snapshot and
        // bind their run to the revision as the active evidence batch (EV-05).
        // A collector failure fails the job but never un-publishes the scan.
        let batch = collect_projects(&observed_graph);
        if !batch.edges.is_empty() || !batch.entities.is_empty() {
            let recorded = graph.record_collector_batch(
                &observed_graph.snapshot.id,
                &batch.run,
                &batch.entities,
                &batch.evidence,
                &batch.edges,
            );
            match recorded {
                Ok(()) => {
                    graph.bind_runs_to_revision(
                        &revision_id,
                        &[(batch.run.run_id.as_str(), "active")],
                    )?;
                }
                Err(error) => {
                    let _ = self.control()?.finish_job(job_id, owner, JobState::Failed);
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }

    /// Gives the ops layer access to the control database (plans, approvals,
    /// operations, recovery) without duplicating the file layout.
    pub fn control_store(&self) -> Result<std::sync::MutexGuard<'_, ControlStore>, EngineError> {
        self.control()
    }

    fn control(&self) -> Result<std::sync::MutexGuard<'_, ControlStore>, EngineError> {
        self.control
            .lock()
            .map_or_else(|_| Err(EngineError::Poisoned), Ok)
    }

    fn graph(&self) -> Result<std::sync::MutexGuard<'_, SqliteSnapshotStore>, EngineError> {
        self.graph
            .lock()
            .map_or_else(|_| Err(EngineError::Poisoned), Ok)
    }

    fn cancellations(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>, EngineError> {
        self.cancellations
            .lock()
            .map_or_else(|_| Err(EngineError::Poisoned), Ok)
    }

    fn require(
        &self,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        match authorizer.decide(principal, permission, scope) {
            diskgraph_core::Decision::Allowed => Ok(()),
            diskgraph_core::Decision::Denied(_) => {
                Err(EngineError::Business(BusinessError::PermissionDenied))
            }
        }
    }
}

/// Measures the engine's storage areas, so a full disk can be attributed to
/// the area that filled up (RT-04).
impl Engine {
    /// Per-area capacity readings with each area's verdict.
    pub fn capacity_report(&self) -> Vec<CapacityReading> {
        let readings = vec![
            (
                StorageArea::GraphDatabase,
                self.data_dir.join("diskgraph.sqlite"),
            ),
            (
                StorageArea::ControlDatabase,
                self.data_dir.join("diskgraph-control.sqlite"),
            ),
            (
                StorageArea::WriteAheadLog,
                self.data_dir.join("diskgraph.sqlite-wal"),
            ),
            (StorageArea::Staging, self.data_dir.join("quarantine")),
            (StorageArea::Backups, self.data_dir.join("backups")),
            (StorageArea::Logs, self.data_dir.join("logs")),
            (StorageArea::Quarantine, self.data_dir.join("quarantine")),
        ];
        readings
            .into_iter()
            .map(|(area, path)| {
                let used_bytes = directory_bytes(&path);
                let verdict = self.capacity_watermark.verdict(used_bytes);
                CapacityReading {
                    area,
                    used_bytes,
                    watermark: self.capacity_watermark,
                    verdict,
                }
            })
            .collect()
    }

    /// Whether the data directory can take new work. A refusal never deletes
    /// anything: existing indexes, control records, and quarantined objects
    /// are left exactly as they are.
    pub fn accepts_new_work(&self) -> bool {
        self.capacity_report()
            .iter()
            .all(|reading| reading.verdict.accepts_new_work())
    }
}

/// Total bytes under a path, or zero when it does not exist.
fn directory_bytes(path: &std::path::Path) -> u64 {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return 0,
    };
    if metadata.is_file() {
        return metadata.len();
    }
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            total = total.saturating_add(directory_bytes(&entry.path()));
        }
    }
    total
}

/// A stable fingerprint of the scan options, so two snapshots can be compared
/// only when they were produced the same way (FS-01).
fn options_fingerprint(options: &diskgraph_disktree_core::scan::ScanOptions) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    options.apparent_size.hash(&mut hasher);
    options.follow_links.hash(&mut hasher);
    options.include_hidden.hash(&mut hasher);
    options.one_filesystem.hash(&mut hasher);
    options.max_depth.hash(&mut hasher);
    options.dedup_hardlinks.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn scan_settings(
    options: &diskgraph_disktree_core::scan::ScanOptions,
) -> diskgraph_core::ScanSettings {
    diskgraph_core::ScanSettings {
        apparent_size: options.apparent_size,
        follow_links: options.follow_links,
        include_hidden: options.include_hidden,
        one_filesystem: options.one_filesystem,
        max_depth: options.max_depth,
        dedup_hardlinks: options.dedup_hardlinks,
    }
}

#[cfg(unix)]
fn locator_volume_id(locator: &Locator) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let path = locator.to_native_path().ok()?;
    std::fs::metadata(path).ok().map(|m| m.dev().to_string())
}

#[cfg(not(unix))]
fn locator_volume_id(_locator: &Locator) -> Option<String> {
    None
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis()
        .try_into()
        .expect("timestamp beyond u64")
}
