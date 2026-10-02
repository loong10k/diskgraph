//! Engine orchestration (P1 tasks 2.4 / 2.8 / 2.9 / 2.12, specs ST-01 /
//! RT-01 / RT-02 / RT-04): authorized scope services, durable scan jobs with
//! owner fencing and cooperative cancellation, staging-based atomic
//! publication, and per-scan budgets. The engine never mutates user files.

mod scan_progress_guard;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use diskgraph_core::{
    Authorizer, BudgetDecision, BudgetUsage, BusinessError, CapacityReading, DiskGraph, Grant,
    Locator, Permission, PolicyAuthorizer, PrincipalId, QueryBudget, ResourceLocator, ResourceRef,
    ScanBudget, ScanBudgetStop, ScanExclusions, ScanWindow, ScopeId, ServerId, StorageArea,
    Watermark,
};
use diskgraph_store::{
    CandidateSelection, ControlStore, JobKind, JobRecord, JobState, ScopeRecord,
    SqliteSnapshotStore, StoreError,
};

mod collectors;
pub mod content;
pub mod live_evidence;
mod queries;
mod relation_queries;
mod runner;
mod scoped_file;
#[cfg(test)]
mod tests;
pub mod verify;
mod verify_limits;
pub use verify_limits::VerifyLimits;

pub use collectors::{
    COLLECTOR_ID, COLLECTOR_VERSION, ProjectBatch, RULE_VERSION, collect_projects,
};
pub use queries::{
    ExploreSummary, ImpactEntry, ImpactResult, Propagation, cursor_context, explore, impact,
    impact_bounded, impact_bounded_with_neighbors, impact_propagation, incompatibility_name,
    search_nodes,
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
    /// 节点总数是否完成；查询期限中断时 0 不代表空 revision。
    pub node_counts_complete: bool,
    pub rows: Vec<CompareRow>,
    pub summary: diskgraph_core::Summary,
    /// 截断时 summary 仅描述已比较条目，不能解释为完整统计。
    pub truncated: Option<&'static str>,
}

/// One path's comparison, owned so a report outlives the graphs it came from.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CompareRow {
    pub path: String,
    pub verdict: diskgraph_core::Verdict,
    pub left_bytes: Option<u64>,
    pub right_bytes: Option<u64>,
    /// Whether the path is a file rather than a directory. Recorded by the
    /// comparison that read the node, because a content pass would otherwise
    /// have to guess from the name, and a directory's verdict is a statement
    /// about what it holds rather than about bytes anyone can hash.
    pub is_file: bool,
    /// The content digest each side hashed to, when the comparison was
    /// verified. Carrying the value rather than only the verdict is what
    /// lets a caller see *why* two files were called identical, and reuse the
    /// hash instead of reading both files again to find that out.
    pub digests: Option<(String, String)>,
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
                    "is_file": row.is_file,
                    "digests": row.digests.as_ref().map(|(left, right)| {
                        serde_json::json!({ "left": left, "right": right })
                    }),
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
            "node_counts_complete": self.node_counts_complete,
            "summary": self.summary,
            "complete": self.truncated.is_none(),
            "summary_is_partial": self.truncated.is_some(),
            "truncation_reason": self.truncated,
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
    /// 可信旧 FFI 数据库文件兼容入口；默认使用 data_dir/diskgraph.sqlite。
    pub graph_database_path: Option<PathBuf>,
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
            graph_database_path: None,
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
    graph_path: PathBuf,
    max_nodes_per_scan: u64,
    scan_budget: ScanBudget,
    capacity_watermark: Watermark,
    max_active_jobs_per_principal: u32,
    scan_options: diskgraph_disktree_core::scan::ScanOptions,
    graph: Mutex<SqliteSnapshotStore>,
    control: Mutex<ControlStore>,
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    scan_progress: Mutex<HashMap<(String, u64), diskgraph_disktree_core::scan::ScanSnapshot>>,
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
        let graph_path = config
            .graph_database_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join("diskgraph.sqlite"));
        let (mut graph, _) = SqliteSnapshotStore::open_with_backup(
            &graph_path,
            &config.data_dir.join("migration_backups"),
        )?;
        let mut control = ControlStore::open(&config.data_dir.join("diskgraph-control.sqlite"))?;
        let server_id = control.ensure_server()?;
        let roots = control
            .list_scopes()?
            .into_iter()
            .map(|scope| {
                (
                    scope.scope_id.as_str().to_owned(),
                    match scope.root.kind {
                        diskgraph_core::LocatorKind::NativePath => {
                            ResourceLocator::NativePath(scope.root.display().to_owned())
                        }
                        diskgraph_core::LocatorKind::DocumentUri => {
                            ResourceLocator::DocumentUri(scope.root.display().to_owned())
                        }
                    },
                )
            })
            .collect::<Vec<_>>();
        graph.backfill_revision_ownership(server_id.as_str(), &roots)?;
        Ok(Self {
            data_dir: config.data_dir,
            graph_path,
            max_nodes_per_scan: config.max_nodes_per_scan,
            scan_budget: config.scan_budget,
            capacity_watermark: config.capacity_watermark,
            max_active_jobs_per_principal: config.max_active_jobs_per_principal,
            scan_options: config.scan_options.clone(),
            graph: Mutex::new(graph),
            control: Mutex::new(control),
            cancellations: Mutex::new(HashMap::new()),
            scan_progress: Mutex::new(HashMap::new()),
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
        let control = self.control()?;
        let permitted = |scope: &ScopeId| -> Result<bool, EngineError> {
            match Self::require_with_control(
                &control,
                authorizer,
                principal,
                &Permission::MetadataRead,
                scope,
            ) {
                Ok(()) => Ok(true),
                Err(EngineError::Business(BusinessError::PermissionDenied)) => Ok(false),
                Err(error) => Err(error),
            }
        };
        let authorizer_is_permissive = permitted(&admin_scope())?;
        let scopes = control.list_scopes()?;
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let mut allowed = Vec::new();
        for scope in scopes {
            if permitted(&scope.scope_id)? {
                allowed.push(scope);
            }
        }
        // A principal whose only grant is administration sees the whole
        // registry; every other principal sees only its own scopes.
        if allowed.is_empty() && authorizer_is_permissive {
            return Ok(control.list_scopes()?);
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
        {
            let control = self.control()?;
            // 保留已获授权调用方对已撤 scope 的既有 conflict/退出码契约。
            // 未获 token 或实时数据库 grant 的主体仍只能得到 permission_denied。
            if control.scope(scope_id)?.revoked
                && matches!(
                    authorizer.decide(principal, &Permission::IndexWrite, scope_id),
                    diskgraph_core::Decision::Allowed
                )
                && (control.policy_state()?.is_none()
                    || matches!(
                        control
                            .authorizer()?
                            .decide(principal, &Permission::IndexWrite, scope_id),
                        diskgraph_core::Decision::Allowed
                    ))
            {
                return Err(EngineError::Store(StoreError::Conflict(format!(
                    "scope {scope_id} is revoked"
                ))));
            }
        }
        self.require(authorizer, principal, &Permission::IndexWrite, scope_id)?;
        if !self.accepts_new_work() {
            return Err(EngineError::Business(BusinessError::ResourceExhausted));
        }
        let mut control = self.control()?;
        if control.scope(scope_id)?.revoked {
            return Err(EngineError::Store(StoreError::Conflict(format!(
                "scope {scope_id} is revoked"
            ))));
        }
        let job = control
            .create_job_with_quota(
                scope_id,
                kind,
                principal,
                u64::from(self.max_active_jobs_per_principal),
            )?
            .ok_or(EngineError::Business(BusinessError::ResourceExhausted))?;
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
        if self.scope(scope_id)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let server = self.server_id()?;
        Ok(self.revision_reader()?.scope_snapshots(
            server.as_str(),
            scope_id.as_str(),
            limit,
            offset,
        )?)
    }

    /// 回收 scope 旧历史；无 apply 时仅预览，引用关系不明确时保守保留。
    pub fn prune_snapshots(
        &self,
        scope_id: &ScopeId,
        keep_last: u64,
        apply: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<diskgraph_store::RevisionRecord>, EngineError> {
        self.require(authorizer, principal, &Permission::IndexWrite, scope_id)?;
        let server = self.server_id()?;
        let mut graph = self.graph()?;
        let mut control = self.control()?;
        if control.scope(scope_id)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Ok(control.with_retention_guard(scope_id, |safe| {
            if safe {
                graph.prune_revisions(server.as_str(), scope_id.as_str(), keep_last, apply)
            } else {
                Ok(Vec::new())
            }
        })?)
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
        self.control()?.request_cancel(job_id)?;
        if let Some(flag) = self.cancellations()?.get(job_id) {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Claims and runs one job to a terminal state, returning the durable
    /// record (RT-01: reconnection queries this instead of the connection).
    pub fn run_job(&self, job_id: &str, owner: &str) -> Result<JobRecord, EngineError> {
        let claimed = self.control()?.claim_job_once(job_id, owner)?;
        let _progress_cleanup = scan_progress_guard::ScanProgressGuard {
            entries: &self.scan_progress,
            key: (job_id.to_owned(), claimed.fencing_token),
        };
        // An expired owner cannot write after the new claim. Reclaim only
        // generations strictly older than the active fencing token.
        self.graph()?
            .clear_stale_job_staging(job_id, claimed.fencing_token)?;
        // 每个认领代次独立取消标志；过期 owner 的标志不能取消新 owner。
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancellations()?
            .insert(job_id.to_owned(), Arc::clone(&cancel));
        // 租约覆盖转换、staging 和 collector 阶段，不能只在扫描进度循环续租。
        let outcome = std::thread::scope(|threads| {
            let (stop, receiver) = std::sync::mpsc::channel();
            let cancel_ref = &cancel;
            let fence = claimed.fencing_token;
            let keeper = threads.spawn(move || {
                while receiver
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .is_err()
                {
                    if self
                        .control()
                        .and_then(|mut store| {
                            store
                                .heartbeat_fenced(job_id, owner, fence)
                                .map_err(EngineError::from)
                        })
                        .is_err()
                    {
                        cancel_ref.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            });
            let result = self.execute_scan(job_id, owner, claimed.fencing_token, &cancel);
            let _ = stop.send(());
            let _ = keeper.join();
            result
        });
        let final_state = if outcome.is_ok() {
            JobState::Completed
        } else if self
            .control()?
            .cancellation_requested(job_id, claimed.fencing_token)?
        {
            JobState::Cancelled
        } else {
            JobState::Failed
        };
        if outcome.is_err() {
            let staging_id = format!("{job_id}:{}", claimed.fencing_token);
            self.graph()?.clear_staging(&staging_id)?;
        }
        let record =
            self.control()?
                .finish_job_fenced(job_id, owner, claimed.fencing_token, final_state)?;
        self.graph()?
            .clear_stale_job_staging(job_id, claimed.fencing_token)?;
        let mut cancellations = self.cancellations()?;
        if cancellations
            .get(job_id)
            .is_some_and(|current| Arc::ptr_eq(current, &cancel))
        {
            cancellations.remove(job_id);
        }
        drop(cancellations);
        outcome?;
        Ok(record)
    }

    /// 按作业实际 scope 授权读取真实扫描进度；非扫描阶段不捏造计数。
    pub fn job_progress(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<serde_json::Value, EngineError> {
        let job = self.job_status(job_id)?;
        self.require(
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        let entries = self
            .scan_progress
            .lock()
            .map_err(|_| EngineError::Business(BusinessError::InternalError))?;
        let counts=entries.get(&(job_id.to_owned(),job.fencing_token)).map(|p|serde_json::json!({"files":p.files,"directories":p.dirs,"bytes":p.bytes.to_string(),"read_errors":p.errors}));
        Ok(serde_json::json!({"job_id":job_id,"state":job.state,"observed":counts}))
    }

    /// 解析已完成作业实际发布的 revision；认领代次固定，不能返回后来更新的 latest。
    pub fn revision_for_job(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<String, EngineError> {
        let job = self.job_status(job_id)?;
        self.require(
            authorizer,
            principal,
            &Permission::OperationView,
            &job.scope_id,
        )?;
        if job.state != JobState::Completed {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        let revision = format!("rev-{}-{}", job_id, job.fencing_token);
        self.authorize_revision(Some(&job.scope_id), &revision, principal, authorizer)?;
        Ok(revision)
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
        let mut control = self.control()?;
        control.reap_unclaimable_jobs()?;
        Ok(control.list_queued_jobs()?)
    }

    /// The latest published revision for a scope, if it has ever published.
    pub fn latest_revision(&self, scope_id: &ScopeId) -> Result<Option<String>, EngineError> {
        self.control()?.scope(scope_id)?;
        let server = self.server_id()?;
        Ok(self
            .revision_reader()?
            .latest_revision_for_scope(server.as_str(), scope_id.as_str())?)
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
    /// `from_revision` is the source and `to_revision` the side being
    /// corrected. The plan builder reads `from` as the comparison's left, so
    /// swapping them is what `--from b --to a` means: a different question,
    /// not the same one asked backwards.
    pub fn sync_plan(
        &self,
        from_revision: &str,
        to_revision: &str,
        method: diskgraph_core::SyncMethod,
        tolerance_seconds: i64,
    ) -> Result<diskgraph_core::SyncPlan, EngineError> {
        let report = self.compare_revisions(from_revision, to_revision, tolerance_seconds)?;
        // 截断的比较不能生成看似完整的镜像/删除计划。
        if report.truncated.is_some() {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let left_root = self.revision_root_node(from_revision)?;
        let right_root = self.revision_root_node(to_revision)?;
        let entries = report
            .rows
            .iter()
            .map(|row| {
                let path = Path::new(&row.path);
                Ok((
                    self.revision_node_at(from_revision, path)?,
                    self.revision_node_at(to_revision, path)?,
                ))
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let rows = report
            .rows
            .iter()
            .zip(&entries)
            .map(|(row, (left, right))| diskgraph_core::compare::Comparison {
                path: row.path.clone(),
                verdict: row.verdict.clone(),
                left: left.as_ref(),
                right: right.as_ref(),
            })
            .collect::<Vec<_>>();
        let excluded = self.index_paths_in_scope(&left_root);
        Ok(diskgraph_core::build_sync_plan(
            method,
            &rows,
            &native_path(&left_root)?,
            &native_path(&right_root)?,
            &excluded,
        ))
    }

    /// The store's data directory, as paths relative to an indexed root, when
    /// it sits inside that root at all. A store kept outside the tree it
    /// describes - `~/.diskgraph` alongside a project - has nothing to
    /// exclude, because it is not in the comparison to begin with.
    fn index_paths_in_scope(
        &self,
        root: &diskgraph_core::DiskNode,
    ) -> Vec<diskgraph_core::PlanExclusion> {
        let Ok(root_path) = native_path(root) else {
            return Vec::new();
        };
        let data_dir = match std::fs::canonicalize(self.data_dir()) {
            Ok(path) => path,
            Err(_) => return Vec::new(),
        };
        let data_dir = data_dir.to_string_lossy().into_owned();
        let relative = match data_dir
            .strip_prefix(&root_path)
            .map(|rest| rest.trim_start_matches('/'))
        {
            Some(relative) if !relative.is_empty() => relative,
            _ => return Vec::new(),
        };
        vec![diskgraph_core::PlanExclusion {
            path: relative.to_owned(),
            reason: diskgraph_core::ExcludeReason::IndexData,
        }]
    }

    /// 以有序路径游标比较两份历史，不同时加载两棵完整树；截断时统计明确为局部。
    pub fn compare_revisions(
        &self,
        left_revision: &str,
        right_revision: &str,
        tolerance_seconds: i64,
    ) -> Result<ComparisonReport, EngineError> {
        self.compare_revisions_bounded(
            left_revision,
            right_revision,
            tolerance_seconds,
            diskgraph_core::QueryBudget::default(),
        )
    }

    /// 使用指定节点、时间和响应预算执行历史比较，返回已逐条产出的结果。
    pub fn compare_revisions_bounded(
        &self,
        left_revision: &str,
        right_revision: &str,
        tolerance_seconds: i64,
        budget: diskgraph_core::QueryBudget,
    ) -> Result<ComparisonReport, EngineError> {
        let budget = budget.validated()?;
        let left = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        let right = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        let left_snapshot = left.revision(left_revision)?.snapshot_id;
        let right_snapshot = right.revision(right_revision)?.snapshot_id;
        let left_root = left
            .root_node(&left_snapshot)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let right_root = right
            .root_node(&right_snapshot)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let left_path = native_path(&left_root)?;
        let right_path = native_path(&right_root)?;
        let started = std::time::Instant::now();
        let mut rows = Vec::new();
        let mut summary = diskgraph_core::Summary::default();
        let result = left.with_ordered_nodes(&left_snapshot, &left_path, |left_iter| {
            right.with_ordered_nodes(&right_snapshot, &right_path, |right_iter| {
                let mut current_left = left_iter.next().transpose()?;
                let mut current_right = right_iter.next().transpose()?;

                let mut bytes = 0usize;
                let mut reason = None;
                while current_left.is_some() || current_right.is_some() {
                    if rows.len() >= budget.max_nodes {
                        reason = Some("node_limit");
                        break;
                    }
                    if started.elapsed().as_millis() >= u128::from(budget.deadline_ms) {
                        reason = Some("deadline");
                        break;
                    }
                    let ordering = match (&current_left, &current_right) {
                        (Some(left), Some(right)) => left.0.cmp(&right.0),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        _ => break,
                    };
                    let on_left = if ordering != std::cmp::Ordering::Greater {
                        current_left.take()
                    } else {
                        None
                    };
                    let on_right = if ordering != std::cmp::Ordering::Less {
                        current_right.take()
                    } else {
                        None
                    };
                    let path = on_left
                        .as_ref()
                        .or(on_right.as_ref())
                        .expect("at least one node exists")
                        .0
                        .clone();
                    let verdict = match (&on_left, &on_right) {
                        (Some(left), Some(right)) => diskgraph_core::compare::compare_entry(
                            &left.1,
                            &right.1,
                            tolerance_seconds,
                        ),
                        (Some(_), None) => diskgraph_core::Verdict::LeftOnly,
                        _ => diskgraph_core::Verdict::RightOnly,
                    };
                    let row = CompareRow {
                        path,
                        verdict,
                        left_bytes: on_left.as_ref().map(|node| node.1.subtree_bytes),
                        right_bytes: on_right.as_ref().map(|node| node.1.subtree_bytes),
                        is_file: on_left
                            .as_ref()
                            .or(on_right.as_ref())
                            .is_some_and(|node| node.1.kind == diskgraph_core::NodeKind::File),
                        digests: None,
                    };
                    let size = serde_json::to_vec(&row)?.len();
                    if bytes.saturating_add(size) > budget.max_response_bytes {
                        reason = Some("response_byte_limit");
                        break;
                    }
                    bytes += size;
                    match &row.verdict {
                        diskgraph_core::Verdict::LeftOnly => summary.left_only += 1,
                        diskgraph_core::Verdict::RightOnly => summary.right_only += 1,
                        diskgraph_core::Verdict::Same { .. } => summary.same += 1,
                        diskgraph_core::Verdict::Different { reason } => {
                            summary.different += 1;
                            if *reason == diskgraph_core::DifferentReason::UnknownSize {
                                summary.unknown += 1;
                            }
                        }
                    }
                    rows.push(row);
                    if ordering != std::cmp::Ordering::Greater {
                        current_left = left_iter.next().transpose()?;
                    }
                    if ordering != std::cmp::Ordering::Less {
                        current_right = right_iter.next().transpose()?;
                    }
                }
                Ok(reason)
            })
        });
        let mut truncated = match result {
            Ok(reason) => reason,
            Err(error) if error.is_interrupted() => Some("deadline"),
            Err(error) => return Err(error.into()),
        };
        let mut node_counts_complete = true;
        let mut count =
            |store: &SqliteSnapshotStore, snapshot: &str| -> Result<usize, EngineError> {
                match store.node_count(snapshot) {
                    Ok(value) => Ok(value as usize),
                    Err(error) if error.is_interrupted() => {
                        node_counts_complete = false;
                        truncated = Some("deadline");
                        Ok(0)
                    }
                    Err(error) => Err(error.into()),
                }
            };
        let left_nodes = count(&left, &left_snapshot)?;
        let right_nodes = count(&right, &right_snapshot)?;
        Ok(ComparisonReport {
            left_revision: left_revision.to_owned(),
            right_revision: right_revision.to_owned(),
            left_root: left_root.locator,
            right_root: right_root.locator,
            left_nodes,
            right_nodes,
            node_counts_complete,
            rows,
            summary,
            truncated,
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

    /// Reads one bounded relation page after resolving the revision's real scope.
    #[allow(clippy::too_many_arguments)] // Mirrors the existing related() API plus a page cursor.
    pub fn related_page(
        &self,
        revision_id: &str,
        entity_id: &str,
        outgoing: bool,
        after_edge_id: Option<&str>,
        limit: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(Vec<diskgraph_core::RelationEdge>, bool), EngineError> {
        let graph = self.revision_reader()?;
        self.authorize_revision_with_reader(&graph, None, revision_id, principal, authorizer)?;
        let revision = graph.revision(revision_id)?;
        if outgoing {
            Ok(graph.edges_from_page(&revision.snapshot_id, entity_id, after_edge_id, limit)?)
        } else {
            Ok(graph.edges_to_page(&revision.snapshot_id, entity_id, after_edge_id, limit)?)
        }
    }

    /// 一次影响查询共享一个授权结果与只读连接，避免每个实体重新打开数据库。
    pub fn revision_impact(
        &self,
        revision_id: &str,
        entity_id: &str,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ImpactResult, EngineError> {
        let reader = self.revision_reader()?;
        self.authorize_revision_with_reader(&reader, None, revision_id, principal, authorizer)?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let mut interrupted = false;
        let mut answer = impact_bounded_with_neighbors::<EngineError, _>(
            entity_id,
            budget,
            |current, outgoing, limit| {
                let page = if outgoing {
                    reader.edges_from_page(&snapshot_id, current, None, limit as u64)
                } else {
                    reader.edges_to_page(&snapshot_id, current, None, limit as u64)
                };
                let (edges, more) = match page {
                    Ok(page) => page,
                    Err(error) if error.is_interrupted() => {
                        interrupted = true;
                        return Ok((Vec::new(), true));
                    }
                    Err(error) => return Err(error.into()),
                };
                Ok((
                    edges
                        .into_iter()
                        .map(|edge| {
                            let neighbour = if outgoing {
                                edge.target_entity_id
                            } else {
                                edge.source_entity_id
                            };
                            (neighbour, edge.relation)
                        })
                        .collect(),
                    more,
                ))
            },
        )?;
        if interrupted {
            answer.truncated = Some(diskgraph_core::TruncationReason::Deadline);
        }
        Ok(answer)
    }

    /// 按 revision 的持久归属授权，再通过请求专用只读连接选择有界候选。
    pub fn review_candidates(
        &self,
        revision_id: &str,
        target_bytes: u64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<CandidateSelection, EngineError> {
        let reader = self.revision_reader()?;
        self.authorize_revision_with_reader(&reader, None, revision_id, principal, authorizer)?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        Ok(reader.candidate_selection(&snapshot_id, target_bytes, budget)?)
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
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        self.authorize_revision(None, revision_id, principal, authorizer)
            .map(|_| ())
    }

    /// 按持久化归属解析 revision 并授权；客户端 scope 只能作为一致性断言。
    pub fn authorize_revision(
        &self,
        expected_scope: Option<&ScopeId>,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        let reader = self.revision_reader()?;
        self.authorize_revision_with_reader(
            &reader,
            expected_scope,
            revision_id,
            principal,
            authorizer,
        )
    }

    fn authorize_revision_with_reader(
        &self,
        reader: &SqliteSnapshotStore,
        expected_scope: Option<&ScopeId>,
        revision_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ScopeId, EngineError> {
        let ownership = reader.revision_ownership(revision_id)?;
        let (server, scope) =
            ownership.ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
        let scope = ScopeId::new(scope)
            .map_err(|_| EngineError::Business(BusinessError::PermissionDenied))?;
        if server != self.server_id()?.as_str()
            || expected_scope.is_some_and(|expected| expected != &scope)
            || self.scope(&scope)?.revoked
        {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        self.require(authorizer, principal, &Permission::MetadataRead, &scope)?;
        Ok(scope)
    }

    /// 旧 snapshot API 经 revision 的实际归属授权；未绑定历史明确要求重新索引。
    pub fn authorize_snapshot(
        &self,
        snapshot_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(), EngineError> {
        let revision = self
            .revision_reader()?
            .revision_for_snapshot(snapshot_id)?
            .ok_or(EngineError::Business(BusinessError::PermissionDenied))?;
        self.authorize_revision(None, &revision, principal, authorizer)?;
        Ok(())
    }

    /// 请求专用只读连接，不持有共享写锁。
    pub fn revision_reader(&self) -> Result<SqliteSnapshotStore, EngineError> {
        Ok(SqliteSnapshotStore::open_reader(
            &self.graph_path,
            1000,
            None,
        )?)
    }

    /// 创建带会话取消标志的独立只读连接；关闭会话会中断 SQLite 执行。
    pub fn revision_reader_with_cancel(
        &self,
        cancel: Arc<AtomicBool>,
    ) -> Result<SqliteSnapshotStore, EngineError> {
        Ok(SqliteSnapshotStore::open_reader(
            &self.graph_path,
            1000,
            Some(cancel),
        )?)
    }

    /// 读取 revision 的快照元信息，避免加载所有节点。
    pub fn revision_snapshot(
        &self,
        revision_id: &str,
    ) -> Result<diskgraph_core::DiskSnapshot, EngineError> {
        let reader = self.revision_reader()?;
        Ok(reader.snapshot(&reader.revision(revision_id)?.snapshot_id)?)
    }

    /// 读取指定 revision 中单个节点。
    pub fn revision_node(
        &self,
        revision_id: &str,
        node_id: u64,
    ) -> Result<diskgraph_core::DiskNode, EngineError> {
        let reader = self.revision_reader()?;
        reader
            .node(&reader.revision(revision_id)?.snapshot_id, node_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))
    }

    /// 读取目录的一页，保留 offset 和未知大小计数兼容字段。
    pub fn revision_children_page(
        &self,
        revision_id: &str,
        parent_id: u64,
        minimum: Option<u64>,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::DiskNode>, Option<u64>, u64), EngineError> {
        let reader = self.revision_reader()?;
        Ok(reader.children_page(
            &reader.revision(revision_id)?.snapshot_id,
            parent_id,
            minimum,
            offset,
            limit.min(100),
        )?)
    }

    /// 读取未知大小子节点的一页；调用方必须先完成 revision 授权。
    pub fn revision_unknown_children_page(
        &self,
        revision_id: &str,
        parent_id: u64,
        offset: u64,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::DiskNode>, Option<u64>), EngineError> {
        let reader = self.revision_reader()?;
        Ok(reader.unknown_children_page(
            &reader.revision(revision_id)?.snapshot_id,
            parent_id,
            offset,
            limit.min(100),
        )?)
    }

    /// 读取最大子节点，不为少量 CLI 结果解码完整 revision。
    pub fn revision_top(
        &self,
        revision_id: &str,
        parent_id: u64,
        limit: u64,
    ) -> Result<(Vec<diskgraph_core::DiskNode>, bool), EngineError> {
        let reader = self.revision_reader()?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let limit = limit.min(100);
        let mut items = reader.top(&snapshot_id, parent_id, limit.saturating_add(1))?;
        let more = items.len() as u64 > limit;
        items.truncate(limit as usize);
        Ok((items, more))
    }

    /// The root node of a published revision - the entry a du-style summary
    /// reads its total from. One row, no graph materialization.
    pub fn revision_root_node(
        &self,
        revision_id: &str,
    ) -> Result<diskgraph_core::DiskNode, EngineError> {
        let graph = self.revision_reader()?;
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
        let graph = self.revision_reader()?;
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
            let graph = self.revision_reader()?;
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

    /// 有界历史变化，字段兼容；截断统计明确标为部分结果。调用方须先授权两个 revision。
    pub fn revision_changes(
        &self,
        before: &str,
        after: &str,
    ) -> Result<serde_json::Value, EngineError> {
        let previous = self.revision_snapshot(before)?;
        let current = self.revision_snapshot(after)?;
        let incompatible = if previous.root != current.root {
            Some("different_root")
        } else if previous.volume_id.is_none() || current.volume_id.is_none() {
            Some("unknown_volume")
        } else if previous.volume_id != current.volume_id {
            Some("different_volume")
        } else if previous.settings != current.settings {
            Some("different_settings")
        } else if previous.captured_at_unix_ms > current.captured_at_unix_ms {
            Some("out_of_order")
        } else if !previous.coverage.complete || !current.coverage.complete {
            Some("incomplete_coverage")
        } else {
            None
        };
        if let Some(reason) = incompatible {
            return Ok(
                serde_json::json!({"incompatible":reason,"added":0,"removed":0,"size_changed":0,"complete":true,"summary_is_partial":false}),
            );
        }
        let report = self.compare_revisions(before, after, 0)?;
        Ok(serde_json::json!({
            "incompatible": null,
            "added": report.summary.right_only,
            "removed": report.summary.left_only,
            "size_changed": report.rows.iter().filter(|row| row.left_bytes.is_some() && row.right_bytes.is_some() && row.left_bytes != row.right_bytes).count(),
            "complete": report.truncated.is_none(),
            "summary_is_partial": report.truncated.is_some(),
            "truncation_reason": report.truncated,
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
        let (node, children, _) = self.revision_layer_page(revision_id, parent_id, 0, limit)?;
        Ok((node, children))
    }

    /// A directory page with an explicit next-page indicator for wide folders.
    pub fn revision_layer_page(
        &self,
        revision_id: &str,
        parent_id: u64,
        offset: u64,
        limit: usize,
    ) -> Result<
        (
            diskgraph_core::DiskNode,
            Vec<diskgraph_core::DiskNode>,
            bool,
        ),
        EngineError,
    > {
        let graph = self.revision_reader()?;
        let record = graph.revision(revision_id)?;
        let node = graph
            .node(&record.snapshot_id, parent_id)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let mut children = graph.children(
            &record.snapshot_id,
            parent_id,
            offset,
            (limit as u64).saturating_add(1),
        )?;
        let more = children.len() > limit;
        children.truncate(limit);
        Ok((node, children, more))
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
        self.authorize_revision(Some(scope_id), revision_id, principal, authorizer)?;
        self.tree_view_bounded(
            revision_id,
            depth,
            min_bytes,
            diskgraph_core::QueryBudget::default(),
        )
    }

    /// 按层读取树；节点、期限和响应字节预算耗尽时返回明确截断诊断。
    /// 可信内部调用必须先经 authorize_revision 验证请求主体。
    pub fn tree_view_bounded(
        &self,
        revision_id: &str,
        depth: usize,
        min_bytes: u64,
        budget: diskgraph_core::QueryBudget,
    ) -> Result<diskgraph_core::TreeView, EngineError> {
        let budget = budget.validated()?;
        let reader = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        let snapshot = reader.revision(revision_id)?.snapshot_id;
        let root = reader
            .root_node(&snapshot)?
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        let root_id = root.id;
        let started = std::time::Instant::now();
        let mut nodes = vec![(root, 0usize)];
        let mut children = HashMap::<u64, Vec<u64>>::new();
        let mut counts = HashMap::<u64, (u64, u64)>::new();
        let mut bytes = serde_json::to_vec(&nodes[0].0)
            .map_err(StoreError::from)?
            .len();
        if bytes > budget.max_response_bytes {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let mut reason = None;
        let mut index = 0;
        while index < nodes.len() {
            let (node, level) = &nodes[index];
            let (id, level) = (node.id, *level);
            if started.elapsed().as_millis() >= u128::from(budget.deadline_ms) {
                reason = Some("deadline");
                break;
            }
            let (all, kept) = match reader.child_counts(&snapshot, id, min_bytes) {
                Ok(counts) => counts,
                Err(error) if error.is_interrupted() => {
                    reason = Some("deadline");
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            counts.insert(id, (all, kept));
            if level >= depth.min(budget.max_depth) {
                index += 1;
                continue;
            }
            if nodes.len() >= budget.max_nodes {
                if kept > 0 {
                    reason = Some("node_limit");
                }
                index += 1;
                continue;
            }
            let remaining = budget.max_nodes - nodes.len();
            let page =
                match reader.children_page(&snapshot, id, Some(min_bytes), 0, remaining as u64) {
                    Ok(page) => page,
                    Err(error) if error.is_interrupted() => {
                        reason = Some("deadline");
                        break;
                    }
                    Err(error) => return Err(error.into()),
                };
            // tree 保留未知大小对象；children_page 的已知大小过滤仅用于普通目录列表。
            let page = if page.2 > 0 {
                match reader.children(&snapshot, id, 0, remaining as u64) {
                    Ok(page) => page,
                    Err(error) if error.is_interrupted() => {
                        reason = Some("deadline");
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            } else {
                page.0
            };
            let mut ids = Vec::new();
            for child in page {
                if child.subtree_bytes < min_bytes {
                    continue;
                }
                let size = serde_json::to_vec(&child).map_err(StoreError::from)?.len();
                if bytes.saturating_add(size) > budget.max_response_bytes {
                    reason = Some("response_byte_limit");
                    break;
                }
                bytes += size;
                ids.push(child.id);
                nodes.push((child, level + 1));
            }
            if (ids.len() as u64) < kept && reason.is_none() {
                reason = Some("node_limit");
            }
            children.insert(id, ids);
            index += 1;
            if reason == Some("response_byte_limit") {
                break;
            }
        }
        let nodes_read = nodes.len();
        let mut rendered = HashMap::<u64, serde_json::Value>::new();
        for (node, level) in nodes.into_iter().rev() {
            let mut value = serde_json::json!({"name":node.name, "kind":node.kind, "size_bytes":node.subtree_bytes, "own_bytes":node.direct_bytes, "files":node.files, "dirs":node.directories});
            if let Some(category) = node.category_hint {
                value["category_hint"] = serde_json::json!(category);
            }
            if node.read_error {
                value["read_error"] = serde_json::json!(true);
            }
            let (all, kept) = counts.get(&node.id).copied().unwrap_or((0, 0));
            let ids = children.remove(&node.id).unwrap_or_default();
            if level < depth.min(budget.max_depth) && all > 0 {
                let shown = ids.len() as u64;
                value["children"] = serde_json::json!(
                    ids.into_iter()
                        .filter_map(|id| rendered.remove(&id))
                        .collect::<Vec<_>>()
                );
                if all > kept {
                    value["hidden_below_min_bytes"] = serde_json::json!(all - kept);
                }
                if shown < kept {
                    value["truncated"] = serde_json::json!(true);
                    value["children_count"] = serde_json::json!(all);
                }
            } else if all > 0 {
                value["truncated"] = serde_json::json!(true);
                value["children_count"] = serde_json::json!(all);
            }
            rendered.insert(node.id, value);
        }
        let mut root = rendered
            .remove(&root_id)
            .ok_or(EngineError::Business(BusinessError::NotFound))?;
        if let Some(reason) = reason {
            root["truncated"] = serde_json::json!(true);
            root["truncation_reason"] = serde_json::json!(reason);
            root["nodes_read"] = serde_json::json!(nodes_read);
        }
        Ok(diskgraph_core::TreeView { root })
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
        fence: u64,
        cancel: &AtomicBool,
    ) -> Result<(), EngineError> {
        let job = self.control()?.job(job_id)?;
        if job.fencing_token != fence || job.owner != owner {
            return Err(StoreError::StaleOwner.into());
        }
        let scope = self.control()?.scope(&job.scope_id)?;
        if scope.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if self.control()?.policy_state()?.is_some() {
            self.require(
                &self.policy_authorizer()?,
                &job.principal,
                &Permission::IndexWrite,
                &job.scope_id,
            )?;
        }
        let root = scope.root.to_native_path().map_err(|error| {
            EngineError::Store(StoreError::InvalidGraph(format!(
                "scope root is not addressable on this platform: {error}"
            )))
        })?;

        // 2.5: the walk is an observation over a window, recorded with the
        // options that produced it, so two snapshots are only comparable when
        // they were configured the same way.
        let started_at_unix_ms = now_ms();
        let mut last_heartbeat = started_at_unix_ms;
        let staging_id = format!("{job_id}:{}", job.fencing_token);
        let options = self.scan_options.clone();
        let handle =
            diskgraph_disktree_core::scan::ScanHandle::spawn(root.clone(), options.clone());
        let mut exceeded = false;
        let tree = loop {
            if now_ms().saturating_sub(last_heartbeat) >= 5000 {
                if self
                    .control()?
                    .heartbeat_fenced(job_id, owner, job.fencing_token)
                    .is_err()
                {
                    handle.cancel();
                    return Err(EngineError::Store(StoreError::StaleOwner));
                }
                last_heartbeat = now_ms();
            }
            let progress = handle.progress.snapshot();
            if let Ok(mut entries) = self.scan_progress.lock() {
                entries.insert((job_id.to_owned(), job.fencing_token), progress.clone());
            }
            if progress.files.saturating_add(progress.dirs)
                > self.max_nodes_per_scan.min(self.scan_budget.max_nodes)
                || now_ms().saturating_sub(started_at_unix_ms) > self.scan_budget.max_duration_ms
            {
                exceeded = true;
                handle.cancel();
            }
            if cancel.load(Ordering::SeqCst)
                || self.control()?.cancellation_requested(job_id, fence)?
                || self.scope(&job.scope_id)?.revoked
                || (self.control()?.policy_state()?.is_some()
                    && !matches!(
                        self.policy_authorizer()?.decide(
                            &job.principal,
                            &Permission::IndexWrite,
                            &job.scope_id
                        ),
                        diskgraph_core::Decision::Allowed
                    ))
            {
                cancel.store(true, Ordering::SeqCst);
                handle.cancel();
            }
            match handle.poll() {
                Some(result) => break result,
                None => thread::sleep(Duration::from_millis(20)),
            }
        };
        if exceeded {
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }
        let tree = tree.map_err(|error| {
            if cancel.load(Ordering::SeqCst) {
                EngineError::Business(BusinessError::Conflict)
            } else {
                EngineError::Io(error)
            }
        })?;
        let scanned = diskgraph_disktree::convert_tree(&root, &tree, scan_settings(&options))
            .map_err(|error| {
                if error.kind() == io::ErrorKind::Unsupported {
                    EngineError::Business(BusinessError::Unsupported)
                } else {
                    EngineError::Io(error)
                }
            })?;
        drop(tree);
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
            let _ = graph.clear_staging(&staging_id);
            return Err(EngineError::Business(BusinessError::BudgetExceeded));
        }

        // 2.9: charge the walk against its budget. A reached limit stops the
        // walk for a named reason instead of returning less data silently, and
        // a cancellation observed here is reported, never swallowed. The byte
        // charge is the node's OWN bytes, never its subtree aggregate: every
        // ancestor would otherwise bill the same file again, so a deep tree
        // would multiply its real size by its depth and stop on phantom bytes.
        for node in &scanned.nodes {
            match self.scan_budget.charge_node(
                &mut usage,
                (serde_json::to_vec(&node.v1)
                    .map_err(StoreError::from)?
                    .len()
                    + node.v1.name.to_lowercase().len()
                    + (match &node.v1.locator {
                        diskgraph_core::ResourceLocator::NativePath(path)
                        | diskgraph_core::ResourceLocator::DocumentUri(path) => path,
                    })
                    .to_lowercase()
                    .len()) as u64,
            ) {
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
            let _ = graph.clear_staging(&staging_id);
            return Err(EngineError::Business(match stop {
                ScanBudgetStop::Cancelled => BusinessError::Conflict,
                _ => BusinessError::BudgetExceeded,
            }));
        }

        // Stage in bounded batches, then publish snapshot + revision + latest
        // pointer in one transaction (ST-01).
        let mut graph = self.graph()?;
        for batch in scanned
            .nodes
            .chunks(self.scan_budget.write_batch_nodes.max(1) as usize)
        {
            if !self.accepts_new_work() {
                graph.clear_staging(&staging_id)?;
                return Err(EngineError::Business(BusinessError::ResourceExhausted));
            }
            self.control()?
                .heartbeat_fenced(job_id, owner, job.fencing_token)?;
            self.control()?
                .with_job_fence(job_id, owner, job.fencing_token, || {
                    graph.append_staging_iter(&staging_id, batch.iter().map(|node| &node.v1))
                })?;
        }
        let v1_nodes: Vec<diskgraph_core::DiskNode> =
            scanned.nodes.into_iter().map(|node| node.v1).collect();
        let revision_id = format!("rev-{}-{}", job_id, job.fencing_token);
        let published_at = now_ms();
        let observed_graph = DiskGraph {
            snapshot: scanned.snapshot,
            nodes: v1_nodes,
            evidence: scanned.evidence,
        };
        if !self.accepts_new_work() {
            graph.clear_staging(&staging_id)?;
            return Err(EngineError::Business(BusinessError::ResourceExhausted));
        }
        // 撤权和发布共用控制库锁，防止检查通过后撤权仍发布。
        let mut control = self.control()?;
        if control.scope(&job.scope_id)?.revoked || cancel.load(Ordering::SeqCst) {
            graph.clear_staging(&staging_id)?;
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        if control.policy_state()?.is_some() {
            Self::require_with_control(
                &control,
                &control.authorizer()?,
                &job.principal,
                &Permission::IndexWrite,
                &job.scope_id,
            )?;
        }
        let server_id = control.ensure_server()?;
        control.heartbeat_fenced(job_id, owner, job.fencing_token)?;
        let result = control.with_job_fence(job_id, owner, job.fencing_token, || {
            graph.publish_revision_owned(
                &staging_id,
                &observed_graph,
                &revision_id,
                published_at,
                Some((server_id.as_str(), job.scope_id.as_str())),
            )
        });
        drop(control);
        if let Err(error) = result {
            let _ = graph.clear_staging(&staging_id);
            return Err(error.into());
        }

        // Deterministic collectors run against the just-published snapshot and
        // bind their run to the revision as the active evidence batch (EV-05).
        // A collector failure fails the job but never un-publishes the scan.
        drop(graph);
        let batch = collect_projects(&observed_graph);
        let mut graph = self.graph()?;
        if !batch.edges.is_empty() || !batch.entities.is_empty() {
            self.control()?
                .with_job_fence(job_id, owner, job.fencing_token, || {
                    graph.record_collector_batch(
                        &observed_graph.snapshot.id,
                        &batch.run,
                        &batch.entities,
                        &batch.evidence,
                        &batch.edges,
                    )?;
                    graph.bind_runs_to_revision(
                        &revision_id,
                        &[(batch.run.run_id.as_str(), "active")],
                    )
                })?;
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
        let control = self.control()?;
        Self::require_with_control(&control, authorizer, principal, permission, scope)
    }

    fn require_with_control(
        control: &diskgraph_store::ControlStore,
        authorizer: &dyn Authorizer,
        principal: &PrincipalId,
        permission: &Permission,
        scope: &ScopeId,
    ) -> Result<(), EngineError> {
        match authorizer.decide(principal, permission, scope) {
            diskgraph_core::Decision::Allowed => {
                // 请求能力只是上限；持久策略存在时，始终与当前数据库授权取交集。
                let denied = if scope == &admin_scope() {
                    control.policy_state()?.is_some()
                        && !matches!(
                            control.authorizer()?.decide(principal, permission, scope),
                            diskgraph_core::Decision::Allowed
                        )
                } else if control.policy_state()?.is_some() {
                    control.live_permission(principal, permission, scope)? == Some(false)
                } else {
                    // 没有持久策略的可信内部兼容入口仍由传入 authorizer 决定。
                    false
                };
                if denied {
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                } else {
                    Ok(())
                }
            }
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
            (StorageArea::GraphDatabase, self.graph_path.clone()),
            (
                StorageArea::ControlDatabase,
                self.data_dir.join("diskgraph-control.sqlite"),
            ),
            (StorageArea::WriteAheadLog, {
                let mut wal = self.graph_path.as_os_str().to_os_string();
                wal.push("-wal");
                PathBuf::from(wal)
            }),
            (StorageArea::Staging, self.data_dir.join("quarantine")),
            (
                StorageArea::Backups,
                self.data_dir.join("migration_backups"),
            ),
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
        self.capacity_watermark
            .verdict(directory_bytes(&self.data_dir))
            .accepts_new_work()
            && self
                .capacity_report()
                .iter()
                .all(|reading| reading.verdict.accepts_new_work())
            && volume_headroom(&self.data_dir).is_some_and(|free| free >= 64 * 1024 * 1024)
    }
}

/// 读取卷的可用字节；容量门禁无法测量时拒绝新工作。
fn volume_headroom(path: &Path) -> Option<u64> {
    diskgraph_disktree_core::space::space_info(path)
        .ok()
        .map(|space| space.available)
}

/// Total bytes under a path, or zero when it does not exist.
fn directory_bytes(path: &std::path::Path) -> u64 {
    let mut pending = vec![path.to_path_buf()];
    let mut total = 0u64;
    while let Some(path) = pending.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return u64::MAX,
        };
        // 容量统计不能沿 quarantine 中的链接扫描 scope 外的数据或循环链接。
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            total = total.saturating_add(metadata.len());
            continue;
        }
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => return u64::MAX,
        };
        for entry in entries {
            match entry {
                Ok(entry) => pending.push(entry.path()),
                Err(_) => return u64::MAX,
            }
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

#[cfg(windows)]
fn locator_volume_id(locator: &Locator) -> Option<String> {
    let path = locator.to_native_path().ok()?;
    diskgraph_disktree_core::space::device_for(&path)
}

#[cfg(not(any(unix, windows)))]
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
