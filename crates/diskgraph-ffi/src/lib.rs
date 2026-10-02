//! UniFFI read-only API. JSON responses keep the first binding contract small.

use std::io::Write;
use std::path::{Path, PathBuf};

use diskgraph_core::{DiskGraph, ResourceLocator};
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod native_service;
mod native_service_error;
pub use native_service::NativeService;
pub use native_service_error::NativeServiceError;

uniffi::setup_scaffolding!();

type ApiResult = Result<Value, String>;
type JobAuthorization = std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
const MAX_QUERY_LIMIT: u32 = 1_000;

/// Reports implemented capabilities; unsupported scopes are never claimed empty.
#[uniffi::export]
pub fn capabilities_json() -> String {
    response(Ok(json!({
        "platform": std::env::consts::OS,
        "native_path_scan": cfg!(any(target_os = "macos", target_os = "windows", target_os = "linux")),
        "document_uri_scan": false,
        "read_only_queries": true,
        "cleanup_execution": false
    })))
}

/// Scans a caller-chosen native directory and persists an immutable snapshot.
#[uniffi::export]
pub fn scan_native_json(database_path: String, root_path: String) -> String {
    response(run_scan_with_cancel(
        &database_path,
        &root_path,
        &std::sync::atomic::AtomicBool::new(false),
    ))
}

/// Returns the latest snapshot ID for a native path previously scanned.
#[uniffi::export]
pub fn latest_native_snapshot_json(database_path: String, root_path: String) -> String {
    response((|| {
        let canonical = Path::new(&root_path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let engine = open_engine(&database_path)?;
        let principal = local_principal()?;
        let policy = engine
            .policy_authorizer()
            .map_err(|error| error.to_string())?;
        let scopes = engine
            .list_scopes(&principal, &policy)
            .map_err(|error| error.to_string())?;
        let scope = scopes
            .into_iter()
            .find(|scope| {
                !scope.revoked && scope.root.to_native_path().ok().as_deref() == Some(&canonical)
            })
            .ok_or_else(|| {
                "scope is unbound or access is denied; register and reindex".to_owned()
            })?;
        let revision = engine
            .latest_revision(&scope.scope_id)
            .map_err(|error| error.to_string())?;
        let id = revision
            .map(|revision| {
                engine
                    .revision_snapshot(&revision)
                    .map(|snapshot| snapshot.id)
            })
            .transpose()
            .map_err(|error| error.to_string())?;
        Ok(json!({ "snapshot_id": id }))
    })())
}

#[uniffi::export]
pub fn top_json(database_path: String, snapshot_id: String, parent_id: u64, limit: u32) -> String {
    response((|| {
        let limit = bounded_limit(limit)?;
        let store = open_store(&database_path, &[&snapshot_id])?;
        let nodes = store
            .top(&snapshot_id, parent_id, u64::from(limit))
            .map_err(|error| error.to_string())?;
        Ok(json!(nodes))
    })())
}

#[uniffi::export]
pub fn children_json(
    database_path: String,
    snapshot_id: String,
    parent_id: u64,
    offset: u64,
    limit: u32,
) -> String {
    response((|| {
        let limit = bounded_limit(limit)?;
        let store = open_store(&database_path, &[&snapshot_id])?;
        let mut nodes = store
            .children(&snapshot_id, parent_id, offset, u64::from(limit) + 1)
            .map_err(|error| error.to_string())?;
        let has_more = nodes.len() > limit as usize;
        nodes.truncate(limit as usize);
        Ok(json!({
            "items": nodes,
            "next_offset": if has_more {
                Some(offset.saturating_add(u64::from(limit)))
            } else {
                None
            }
        }))
    })())
}

#[uniffi::export]
pub fn explain_json(database_path: String, snapshot_id: String, node_id: u64) -> String {
    response((|| {
        let store = open_store(&database_path, &[&snapshot_id])?;
        let node = store
            .node(&snapshot_id, node_id)
            .map_err(|error| error.to_string())?;
        let Some(node) = node else {
            return Ok(Value::Null);
        };
        let evidence = store
            .evidence(&snapshot_id, node_id)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "node": node, "evidence": evidence }))
    })())
}

/// Compares a locator in two complete, compatible snapshots.
#[uniffi::export]
pub fn growth_json(
    database_path: String,
    before_snapshot_id: String,
    after_snapshot_id: String,
    locator_json: String,
) -> String {
    response((|| {
        let locator: ResourceLocator =
            serde_json::from_str(&locator_json).map_err(|error| error.to_string())?;
        let store = open_store(&database_path, &[&before_snapshot_id, &after_snapshot_id])?;
        let load_one = |id: &str| -> Result<DiskGraph, String> {
            Ok(DiskGraph {
                snapshot: store.snapshot(id).map_err(|error| error.to_string())?,
                nodes: store
                    .node_by_locator(id, &locator)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .collect(),
                evidence: Vec::new(),
            })
        };
        let before = load_one(&before_snapshot_id)?;
        let after = load_one(&after_snapshot_id)?;
        let growth = after.growth(&before, &locator);
        Ok(match growth {
            Some(growth) => json!({
                "before": growth.before,
                "after": growth.after,
                "delta_bytes": growth.delta_bytes.to_string()
            }),
            None => Value::Null,
        })
    })())
}

/// Returns candidates for review only, never paths to execute automatically.
#[uniffi::export]
pub fn candidates_json(database_path: String, snapshot_id: String, target_bytes: u64) -> String {
    response((|| {
        let store = open_store(&database_path, &[&snapshot_id])?;
        let selection = store
            .candidate_selection(
                &snapshot_id,
                target_bytes,
                diskgraph_core::QueryBudget::default(),
            )
            .map_err(|error| error.to_string())?;
        if selection.truncated.is_some() {
            return Err(
                "candidate query exceeded budget; use the session API for partial results".into(),
            );
        }
        let candidates: Vec<_> = selection
            .candidates
            .into_iter()
            .map(|(node, evidence)| json!({"node":node,"evidence":evidence}))
            .collect();
        Ok(json!(candidates))
    })())
}

fn local_principal() -> Result<diskgraph_core::PrincipalId, String> {
    diskgraph_core::PrincipalId::new("local-user").map_err(|error| error.to_string())
}

/// Stable on-disk realm for one graph file; sibling graphs never share jobs.
fn realm_dir_for_database(database: &Path) -> Result<PathBuf, String> {
    let database = canonical_database_path(database)?;
    let parent = database.parent().ok_or("graph database has no parent")?;
    Ok(parent
        .join(".diskgraph-realms")
        .join(path_digest(&database)))
}

fn path_digest(path: &Path) -> String {
    let mut hasher = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in path.as_os_str().encode_wide() {
            hasher.update(unit.to_le_bytes());
        }
    }
    #[cfg(not(any(unix, windows)))]
    hasher.update(path.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

fn canonical_database_path(database: &Path) -> Result<PathBuf, String> {
    let parent = database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let parent = parent.canonicalize().map_err(|error| error.to_string())?;
    let name = database
        .file_name()
        .ok_or("graph database needs a file name")?;
    let path = parent.join(name);
    if path.exists() {
        path.canonicalize().map_err(|error| error.to_string())
    } else {
        Ok(path)
    }
}

/// An existing shared control database is reused only if every registered
/// scope resolves to a published revision in this graph. Ambiguous history
/// stays unbound and must be reindexed by an administrator.
fn legacy_realm_matches(database: &Path, legacy: &Path) -> Result<bool, String> {
    if !database.is_file() || !legacy.is_file() {
        return Ok(false);
    }
    let mut control =
        diskgraph_store::ControlStore::open(legacy).map_err(|error| error.to_string())?;
    let scopes = control.list_scopes().map_err(|error| error.to_string())?;
    if scopes.is_empty() {
        return Ok(false);
    }
    let (graph, _) = SqliteSnapshotStore::open_with_backup(
        database,
        &legacy
            .parent()
            .unwrap_or(Path::new("."))
            .join("migration_backups"),
    )
    .map_err(|error| error.to_string())?;
    let server = control.ensure_server().map_err(|error| error.to_string())?;
    for scope in scopes {
        let locator = match scope.root.kind {
            diskgraph_core::LocatorKind::NativePath => {
                ResourceLocator::NativePath(scope.root.display().to_owned())
            }
            diskgraph_core::LocatorKind::DocumentUri => {
                ResourceLocator::DocumentUri(scope.root.display().to_owned())
            }
        };
        let Some(revision) = graph
            .latest_revision_for_root(&locator)
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        if let Some((owner_server, owner_scope)) = graph
            .revision_ownership(&revision)
            .map_err(|error| error.to_string())?
            && (owner_server != server.as_str() || owner_scope != scope.scope_id.as_str())
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn realm_for_database(database: &Path) -> Result<PathBuf, String> {
    let database = canonical_database_path(database)?;
    let parent = database.parent().ok_or("graph database has no parent")?;
    let legacy = parent.join("diskgraph-control.sqlite");
    let marker = parent.join(".diskgraph-legacy-graph");
    let digest = path_digest(&database);
    if let Ok(bound) = std::fs::read_to_string(&marker) {
        if bound.trim() == digest && legacy_realm_matches(&database, &legacy)? {
            return Ok(parent.to_path_buf());
        }
    } else if legacy_realm_matches(&database, &legacy)? {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                file.write_all(digest.as_bytes())
                    .map_err(|error| error.to_string())?;
                file.sync_all().map_err(|error| error.to_string())?;
                return Ok(parent.to_path_buf());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read_to_string(&marker)
                    .map_err(|error| error.to_string())?
                    .trim()
                    == digest
                {
                    return Ok(parent.to_path_buf());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    realm_dir_for_database(&database)
}

fn open_engine(path: &str) -> Result<diskgraph_engine::Engine, String> {
    let database = canonical_database_path(Path::new(path))?;
    let realm = realm_for_database(&database)?;
    let engine = diskgraph_engine::Engine::open(diskgraph_engine::EngineConfig {
        data_dir: realm,
        graph_database_path: Some(database),
        ..Default::default()
    })
    .map_err(|error| error.to_string())?;
    engine
        .bootstrap_local_admin(&local_principal()?)
        .map_err(|error| error.to_string())?;
    Ok(engine)
}

fn open_store(path: &str, snapshots: &[&str]) -> Result<SqliteSnapshotStore, String> {
    let engine = open_engine(path)?;
    let principal = local_principal()?;
    let policy = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    for snapshot in snapshots {
        engine
            .authorize_snapshot(snapshot, &principal, &policy)
            .map_err(|error| format!("{error}; unbound history requires reindexing"))?;
    }
    engine.revision_reader().map_err(|error| error.to_string())
}

fn bounded_limit(limit: u32) -> Result<u32, String> {
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        Err(format!("limit must be between 1 and {MAX_QUERY_LIMIT}"))
    } else {
        Ok(limit)
    }
}

fn response(result: ApiResult) -> String {
    match result {
        Ok(data) => json!({ "schema_version": 1, "ok": true, "data": data }).to_string(),
        Err(error) => json!({ "schema_version": 1, "ok": false, "error": error }).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn read_only_bindings_scan_and_query_native_directory() {
        let root = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".hidden"), b"some data").unwrap();
        let database_path = database.path().join("snapshots.sqlite");
        let database_path = database_path.to_string_lossy().into_owned();
        let root_path = root.path().to_string_lossy().into_owned();
        let scanned: Value =
            serde_json::from_str(&scan_native_json(database_path.clone(), root_path.clone()))
                .unwrap();
        assert_eq!(scanned["ok"], true);
        let snapshot_id = scanned["data"]["snapshot_id"].as_str().unwrap();
        let top: Value =
            serde_json::from_str(&top_json(database_path.clone(), snapshot_id.into(), 1, 10))
                .unwrap();
        assert_eq!(top["ok"], true);
        assert_eq!(top["data"][0]["name"], ".hidden");
        let children: Value = serde_json::from_str(&children_json(
            database_path.clone(),
            snapshot_id.into(),
            1,
            0,
            1,
        ))
        .unwrap();
        assert!(children["data"]["next_offset"].is_null());
        let explained: Value =
            serde_json::from_str(&explain_json(database_path.clone(), snapshot_id.into(), 2))
                .unwrap();
        assert_eq!(explained["data"]["evidence"].as_array().unwrap().len(), 0);
        let candidates: Value = serde_json::from_str(&candidates_json(
            database_path.clone(),
            snapshot_id.into(),
            1,
        ))
        .unwrap();
        assert!(candidates["data"].as_array().unwrap().is_empty());
        std::fs::write(root.path().join("large.bin"), vec![0_u8; 64 * 1024]).unwrap();
        let rescanned: Value =
            serde_json::from_str(&scan_native_json(database_path.clone(), root_path.clone()))
                .unwrap();
        let new_id = rescanned["data"]["snapshot_id"].as_str().unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let locator = serde_json::to_string(&ResourceLocator::NativePath(
            canonical_root.to_string_lossy().into_owned(),
        ))
        .unwrap();
        let growth: Value = serde_json::from_str(&growth_json(
            database_path,
            snapshot_id.into(),
            new_id.into(),
            locator,
        ))
        .unwrap();
        assert_eq!(growth["ok"], true);
        assert!(
            growth["data"]["delta_bytes"]
                .as_str()
                .unwrap()
                .parse::<i128>()
                .unwrap()
                > 0
        );
    }

    #[test]
    fn growth_and_candidates_do_not_decode_unrelated_nodes() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("unrelated.txt"), "data").unwrap();
        let database = data.path().join("graph.sqlite");
        let database_path = database.to_string_lossy().into_owned();
        let root_path = root.path().to_string_lossy().into_owned();
        let scan: Value =
            serde_json::from_str(&scan_native_json(database_path.clone(), root_path)).unwrap();
        let snapshot = scan["data"]["snapshot_id"].as_str().unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute(
            "UPDATE nodes SET kind = 'corrupt' WHERE name = 'unrelated.txt'",
            [],
        )
        .unwrap();
        let locator = serde_json::to_string(&ResourceLocator::NativePath(
            root.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ))
        .unwrap();
        let growth: Value = serde_json::from_str(&growth_json(
            database_path.clone(),
            snapshot.into(),
            snapshot.into(),
            locator,
        ))
        .unwrap();
        assert_eq!(growth["ok"], true, "{growth}");
        assert_eq!(growth["data"]["delta_bytes"], "0");
        let candidates: Value =
            serde_json::from_str(&candidates_json(database_path, snapshot.into(), 1)).unwrap();
        assert_eq!(candidates["ok"], true, "{candidates}");
    }
}

/// A live scan job handle (P7 task 9.2, PF-01). `spawn_scan_json` returns
/// immediately, so a UI thread never blocks on a walk: progress is polled,
/// cancellation is cooperative, and `result_json` is the only joining call.
/// 释放最后一个句柄请求协作取消，后台线程仍会完成清理并记录终态。
#[derive(uniffi::Object)]
pub struct JobHandle {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    state: std::sync::Arc<std::sync::Mutex<JobState>>,
}

struct JobState {
    finished: bool,
    result: Option<Result<serde_json::Value, String>>,
    progress: Option<Value>,
    authorization: Option<JobAuthorization>,
}

#[uniffi::export]
impl JobHandle {
    /// 读取进度而不等待扫描完成；实时授权访问控制库，宿主应在后台线程调用。
    pub fn progress_json(&self) -> String {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (state_name, data): (&str, Option<serde_json::Value>) = if state.finished {
            ("finished", None)
        } else {
            ("running", None)
        };
        let answer = serde_json::json!({
            "state": state_name,
            "result": data,
            "progress": state.progress,
        });
        let authorization = state.authorization.clone();
        drop(state);
        job_response(authorization, Ok(answer))
    }

    /// 不等待扫描完成的结果轮询；运行中返回 None，权限检查应由宿主后台线程执行。
    pub fn poll_result_json(&self) -> Option<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let authorization = state.authorization.clone();
        let finished = state.finished;
        let result = state.result.clone();
        drop(state);
        if let Some(check) = &authorization
            && let Err(error) = check()
        {
            return Some(response(Err(error)));
        }
        if !finished {
            return None;
        }
        Some(job_response(
            authorization,
            result.unwrap_or_else(|| Err("job finished without a result".into())),
        ))
    }

    /// Asks the walk to stop at its next observation boundary. A job that
    /// already finished is unaffected; cancellation is cooperative, so the
    /// final state stays truthful about how far the walk got.
    pub fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Joins the worker and returns the same envelope the synchronous call
    /// would have produced. Idempotent: later calls return the recorded
    /// result without re-running anything.
    pub fn result_json(&self) -> String {
        loop {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.finished {
                let result = state
                    .result
                    .clone()
                    .unwrap_or_else(|| Err("job finished without a result".into()));
                let authorization = state.authorization.clone();
                drop(state);
                return job_response(authorization, result);
            }
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

fn job_response(authorization: Option<JobAuthorization>, result: ApiResult) -> String {
    if let Some(check) = authorization
        && let Err(error) = check()
    {
        return response(Err(error));
    }
    response(result)
}

impl JobHandle {
    fn is_finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finished
    }
}

fn authorized_job_progress(engine: &diskgraph_engine::Engine, job_id: &str) -> ApiResult {
    let principal = local_principal()?;
    let policy = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    let progress = engine
        .job_progress(job_id, &principal, &policy)
        .map_err(|error| error.to_string())?;
    let job = engine
        .job_status(job_id)
        .map_err(|error| error.to_string())?;
    if engine
        .control_store()
        .map_err(|error| error.to_string())?
        .live_permission(
            &principal,
            &diskgraph_core::Permission::MetadataRead,
            &job.scope_id,
        )
        .map_err(|error| error.to_string())?
        == Some(false)
    {
        return Err("permission_denied".into());
    }
    if job.state == diskgraph_store::JobState::Completed {
        engine
            .revision_for_job(job_id, &principal, &policy)
            .map_err(|error| error.to_string())?;
    }
    Ok(progress)
}

/// Spawns a native-path scan on a worker thread and returns a handle at
/// once. The scan uses the same code path as `scan_native_json`; the only
/// difference is who waits.
#[uniffi::export]
pub fn spawn_scan_json(database_path: String, root_path: String) -> std::sync::Arc<JobHandle> {
    spawn_job(move |cancel, progress| {
        let engine = std::sync::Arc::new(open_engine(&database_path)?);
        run_scan_on_engine(engine, &root_path, cancel, progress)
    })
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn spawn_job(
    work: impl FnOnce(
        &std::sync::atomic::AtomicBool,
        &dyn Fn(Value, std::sync::Arc<diskgraph_engine::Engine>),
    ) -> ApiResult
    + Send
    + 'static,
) -> std::sync::Arc<JobHandle> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let state = std::sync::Arc::new(std::sync::Mutex::new(JobState {
        finished: false,
        result: None,
        progress: None,
        authorization: None,
    }));
    let worker_cancel = cancel.clone();
    let worker_state = state.clone();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            work(&worker_cancel, &|value, engine| {
                let mut state = worker_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.authorization.is_none()
                    && let Some(job_id) = value["job_id"].as_str().map(str::to_owned)
                {
                    state.authorization = Some(std::sync::Arc::new(move || {
                        authorized_job_progress(&engine, &job_id).map(|_| ())
                    }));
                }
                state.progress = Some(value);
            })
        }))
        .unwrap_or_else(|_| Err("scan worker crashed".into()));
        let mut state = worker_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.result = Some(outcome);
        state.finished = true;
    });
    std::sync::Arc::new(JobHandle { cancel, state })
}

/// The worker body: the synchronous scan, run to completion unless the
/// cancel flag is observed before the scan starts.
fn run_scan_with_cancel(
    database_path: &str,
    root_path: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<serde_json::Value, String> {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("cancelled before it started".into());
    }
    // The engine's own scan bridge carries cancellation between batches;
    // the FFI layer drives it through the same engine path the server uses
    // so a cancelled job never half-publishes.
    let engine = std::sync::Arc::new(open_engine(database_path)?);
    run_scan_on_engine(engine, root_path, cancel, &|_, _| {})
}

fn run_scan_on_engine(
    engine: std::sync::Arc<diskgraph_engine::Engine>,
    root_path: &str,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &dyn Fn(Value, std::sync::Arc<diskgraph_engine::Engine>),
) -> ApiResult {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("cancelled before it started".into());
    }
    let principal = local_principal()?;
    let authorizer = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    let scope = engine
        .register_scope(std::path::Path::new(root_path), &principal, &authorizer)
        .map_err(|error| error.to_string())?;
    // Scope-local grants exist only after registration, so the authorizer
    // must be reloaded before the job is submitted.
    let authorizer = engine
        .policy_authorizer()
        .map_err(|error| error.to_string())?;
    let job = engine
        .index_scope(&scope, &principal, &authorizer)
        .map_err(|error| error.to_string())?;
    progress(
        serde_json::json!({"job_id":job.job_id,"state":job.state,"observed":null}),
        engine.clone(),
    );
    // The job is claimed and executed on its own thread, while this worker
    // polls: it mirrors the FFI cancel flag into the engine's cancellation
    // channel and collects the final state.
    let runner_engine = std::sync::Arc::clone(&engine);
    let runner_job = job.job_id.clone();
    let mut runner = Some(std::thread::spawn(move || {
        runner_engine.run_job(&runner_job, "ffi-worker")
    }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    let finished = loop {
        let authorizer = engine
            .policy_authorizer()
            .map_err(|error| error.to_string())?;
        let value = authorized_job_progress(&engine, &job.job_id);
        match value {
            Ok(value) => progress(value, engine.clone()),
            Err(error) => {
                progress(Value::Null, engine.clone());
                let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
                return Err(error);
            }
        }
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
        }
        if runner.as_ref().is_some_and(|thread| thread.is_finished()) {
            let outcome = runner
                .take()
                .unwrap()
                .join()
                .map_err(|_| "scan worker crashed".to_owned())?;
            match outcome {
                // 同 scope 请求共享 durable job；认领冲突只表示另一个活 owner 正在完成它。
                Err(diskgraph_engine::EngineError::Store(
                    diskgraph_store::StoreError::StaleOwner,
                )) => {}
                Err(error) => {
                    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err("the scan was cancelled by the caller".into());
                    }
                    return Err(error.to_string());
                }
                Ok(_) => {}
            }
        }
        let current = engine
            .job_status(&job.job_id)
            .map_err(|error| error.to_string())?;
        if matches!(
            current.state,
            diskgraph_store::JobState::Completed
                | diskgraph_store::JobState::Failed
                | diskgraph_store::JobState::Cancelled
        ) {
            break current.state;
        }
        if std::time::Instant::now() >= deadline {
            let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
            return Err("scan wait deadline exceeded".into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as u64;
        if runner.is_none()
            && (current.state == diskgraph_store::JobState::Queued
                || current.lease_expires_unix_ms <= now)
        {
            let claiming = engine.clone();
            let id = job.job_id.clone();
            runner = Some(std::thread::spawn(move || {
                claiming.run_job(&id, "ffi-worker")
            }));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    match finished {
        diskgraph_store::JobState::Completed => {
            let authorizer = engine
                .policy_authorizer()
                .map_err(|error| error.to_string())?;
            let revision = engine
                .revision_for_job(&job.job_id, &principal, &authorizer)
                .map_err(|error| error.to_string())?;
            let snapshot = engine
                .revision_snapshot(&revision)
                .map_err(|error| error.to_string())?;
            let node_count = engine
                .revision_reader()
                .map_err(|error| error.to_string())?
                .node_count(&snapshot.id)
                .map_err(|error| error.to_string())?;
            engine
                .revision_for_job(
                    &job.job_id,
                    &principal,
                    &engine
                        .policy_authorizer()
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({
                "revision": revision,
                "snapshot_id": snapshot.id,
                "node_count": node_count,
                "coverage": snapshot.coverage,
            }))
        }
        other => Err(format!("scan ended as {other:?}")),
    }
}

#[cfg(test)]
mod async_tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn a_spawned_scan_returns_a_handle_at_once_and_join_later() {
        let root = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("f.bin"), vec![0; 4096]).unwrap();
        let database_path = database.path().join("snapshots.sqlite");
        let database_path = database_path.to_string_lossy().into_owned();
        let root_path = root.path().to_string_lossy().into_owned();

        let handle = spawn_scan_json(database_path.clone(), root_path.clone());
        // The UI-thread contract: the call above returned before any work
        // finished, and polling never blocks.
        let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
        assert_eq!(progress["ok"], true);
        assert!(progress["data"]["state"] == "running" || progress["data"]["state"] == "finished");

        // The join produces the same envelope as the synchronous path.
        let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
        assert_eq!(result["ok"], true, "{}", result);
        assert!(result["data"]["node_count"].as_u64().unwrap() >= 2);
        assert!(
            result["data"]["coverage"]["complete"].as_bool().unwrap(),
            "a completed local scan must be complete"
        );
        // Polling after completion is stable, and the result is idempotent.
        let again: Value = serde_json::from_str(&handle.result_json()).unwrap();
        assert_eq!(again, result);
        let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
        assert_eq!(progress["data"]["state"], "finished");
    }

    #[test]
    fn a_cancelled_job_reports_honestly_instead_of_half_publishing() {
        let root = tempfile::tempdir().unwrap();
        let database = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("f.bin"), vec![0; 1024]).unwrap();
        let database_path = database.path().join("snapshots.sqlite");
        let database_path = database_path.to_string_lossy().into_owned();
        let root_path = root.path().to_string_lossy().into_owned();

        let handle = spawn_scan_json(database_path.clone(), root_path.clone());
        handle.cancel();
        let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
        // Either the job finished before the flag landed (honest success) or
        // it reports cancellation (honest refusal); a half-published graph
        // is the one outcome this contract forbids.
        if result["ok"].as_bool().unwrap() {
            assert!(
                result["data"]["coverage"]["complete"].as_bool().unwrap(),
                "an honest success must carry complete coverage"
            );
        } else {
            let error = result["error"].as_str().unwrap();
            assert!(
                error.contains("Cancelled") || error.contains("cancelled"),
                "{error}"
            );
        }
    }

    #[test]
    fn the_ffi_layer_is_independent_of_any_host_application() {
        // 9.7 (PF-02, AI-01): with no host database, no host configuration,
        // and no PruneX on the machine, the full loop still works from an
        // empty directory.
        let host = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("report.txt"), b"standalone").unwrap();
        let database_path = host.path().join("snapshots.sqlite");
        let database_path = database_path.to_string_lossy().into_owned();
        let root_path = root.path().to_string_lossy().into_owned();

        let scanned: Value =
            serde_json::from_str(&scan_native_json(database_path.clone(), root_path.clone()))
                .unwrap();
        assert_eq!(scanned["ok"], true);
        let snapshot_id = scanned["data"]["snapshot_id"].as_str().unwrap();
        let top: Value =
            serde_json::from_str(&top_json(database_path.clone(), snapshot_id.into(), 1, 10))
                .unwrap();
        assert_eq!(top["ok"], true);
        // The read-only surface is intact, and nothing outside the two
        // caller-named directories was touched.
        assert!(host.path().join("snapshots.sqlite").is_file());
        assert!(root.path().join("report.txt").is_file());
    }
}

#[cfg(test)]
mod authorization_tests {
    #[cfg(unix)]
    #[test]
    fn realm_identity_does_not_collapse_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let first = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/graph-\xff"));
        let second = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/graph-\xfe"));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(super::path_digest(first), super::path_digest(second));
    }

    #[test]
    fn a_proven_legacy_control_database_remains_readable() {
        let data = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("old"), b"legacy").unwrap();
        let database = data.path().join("snapshots.sqlite");
        let engine = diskgraph_engine::Engine::open(diskgraph_engine::EngineConfig {
            data_dir: data.path().to_path_buf(),
            graph_database_path: Some(database.clone()),
            ..Default::default()
        })
        .unwrap();
        let principal = super::local_principal().unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(
                root.path(),
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine.run_job(&job.job_id, "legacy-worker").unwrap();
        let old_snapshot = engine
            .revision_snapshot(&engine.latest_revision(&scope).unwrap().unwrap())
            .unwrap()
            .id;
        drop(engine);
        let result: serde_json::Value = serde_json::from_str(&super::latest_native_snapshot_json(
            database.to_string_lossy().into_owned(),
            root.path().to_string_lossy().into_owned(),
        ))
        .unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["snapshot_id"], old_snapshot);
        assert!(data.path().join(".diskgraph-legacy-graph").is_file());
    }

    #[test]
    fn ambiguous_legacy_control_is_not_assigned_to_either_graph() {
        let data = tempfile::tempdir().unwrap();
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        std::fs::write(first_root.path().join("a"), b"a").unwrap();
        std::fs::write(second_root.path().join("b"), b"b").unwrap();
        let principal = super::local_principal().unwrap();
        let first_db = data.path().join("first.sqlite");
        let second_db = data.path().join("second.sqlite");
        for (database, root) in [(&first_db, &first_root), (&second_db, &second_root)] {
            let engine = diskgraph_engine::Engine::open(diskgraph_engine::EngineConfig {
                data_dir: data.path().to_path_buf(),
                graph_database_path: Some(database.clone()),
                ..Default::default()
            })
            .unwrap();
            engine.bootstrap_local_admin(&principal).unwrap();
            let scope = engine
                .register_scope(
                    root.path(),
                    &principal,
                    &engine.policy_authorizer().unwrap(),
                )
                .unwrap();
            let job = engine
                .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
                .unwrap();
            engine.run_job(&job.job_id, "legacy-worker").unwrap();
        }
        for (database, root) in [(&first_db, &first_root), (&second_db, &second_root)] {
            let result: serde_json::Value =
                serde_json::from_str(&super::latest_native_snapshot_json(
                    database.to_string_lossy().into_owned(),
                    root.path().to_string_lossy().into_owned(),
                ))
                .unwrap();
            assert_eq!(result["ok"], false, "{result}");
        }
        assert!(!data.path().join(".diskgraph-legacy-graph").exists());
    }

    #[test]
    fn graph_files_in_one_directory_have_distinct_control_realms() {
        let data = tempfile::tempdir().unwrap();
        let first_root = tempfile::tempdir().unwrap();
        let second_root = tempfile::tempdir().unwrap();
        std::fs::write(first_root.path().join("a"), b"a").unwrap();
        std::fs::write(second_root.path().join("b"), b"b").unwrap();
        let first_db = data.path().join("first.sqlite");
        let second_db = data.path().join("second.sqlite");
        for (database, root) in [(&first_db, &first_root), (&second_db, &second_root)] {
            let result: serde_json::Value = serde_json::from_str(&super::scan_native_json(
                database.to_string_lossy().into_owned(),
                root.path().to_string_lossy().into_owned(),
            ))
            .unwrap();
            assert_eq!(result["ok"], true, "{result}");
        }
        let realms = data.path().join(".diskgraph-realms");
        let entries = std::fs::read_dir(&realms).unwrap().count();
        assert_eq!(entries, 2);
        for database in [&first_db, &second_db] {
            let realm = super::realm_dir_for_database(database).unwrap();
            let control =
                diskgraph_store::ControlStore::open(&realm.join("diskgraph-control.sqlite"))
                    .unwrap();
            assert_eq!(control.list_scopes().unwrap().len(), 1);
        }
    }

    #[test]
    fn legacy_reads_do_not_bypass_scope_revocation() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), [0]).unwrap();
        let database = data
            .path()
            .join("snapshots.sqlite")
            .to_string_lossy()
            .into_owned();
        let scanned: serde_json::Value = serde_json::from_str(&super::scan_native_json(
            database.clone(),
            root.path().to_string_lossy().into_owned(),
        ))
        .unwrap();
        assert_eq!(scanned["ok"], true);
        let realm = super::realm_dir_for_database(std::path::Path::new(&database)).unwrap();
        let mut control =
            diskgraph_store::ControlStore::open(&realm.join("diskgraph-control.sqlite")).unwrap();
        let scopes = control.list_scopes().unwrap();
        assert_eq!(
            scopes.len(),
            1,
            "FFI scan must register its authorization scope"
        );
        control.revoke_scope(&scopes[0].scope_id).unwrap();
        let result: serde_json::Value = serde_json::from_str(&super::top_json(
            database,
            scanned["data"]["snapshot_id"].as_str().unwrap().to_owned(),
            1,
            10,
        ))
        .unwrap();
        assert_eq!(result["ok"], false);
    }
}

#[cfg(test)]
mod native_service_tests {
    use super::*;
    #[test]
    fn persistent_service_queries_deny_revoked_scope_and_closed_session() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), b"data").unwrap();
        let path = data
            .path()
            .join("graph.sqlite")
            .to_string_lossy()
            .into_owned();
        let scan: Value = serde_json::from_str(&scan_native_json(
            path.clone(),
            root.path().to_string_lossy().into_owned(),
        ))
        .unwrap();
        let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
        let service = NativeService::new(path).unwrap();
        let answer: Value = serde_json::from_str(&service.node_json(snapshot.clone(), 1)).unwrap();
        assert_eq!(answer["ok"], true);
        let policy = service.engine.policy_authorizer().unwrap();
        let revision = scan["data"]["revision"].as_str().unwrap();
        let scope = service
            .engine
            .authorize_revision(None, revision, &local_principal().unwrap(), &policy)
            .unwrap();
        service
            .engine
            .revoke_scope(&scope, &local_principal().unwrap(), &policy)
            .unwrap();
        let denied: Value = serde_json::from_str(&service.node_json(snapshot.clone(), 1)).unwrap();
        assert_eq!(denied["ok"], false);
        service.shutdown();
        let closed: Value = serde_json::from_str(&service.node_json(snapshot, 1)).unwrap();
        assert_eq!(closed["ok"], false);
        assert!(closed["error"].as_str().unwrap().contains("closed"));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn session_children_clip_bytes_and_advance_by_actual_page_size() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let leaf = root
            .path()
            .join("x".repeat(180))
            .join("y".repeat(180))
            .join("z".repeat(180));
        std::fs::create_dir_all(&leaf).unwrap();
        for n in 0..100 {
            std::fs::write(leaf.join(format!("file-{n:03}")), b"x").unwrap();
        }
        let path = data
            .path()
            .join("graph.sqlite")
            .to_string_lossy()
            .into_owned();
        let scan: Value = serde_json::from_str(&scan_native_json(
            path.clone(),
            root.path().to_string_lossy().into_owned(),
        ))
        .unwrap();
        let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
        let db = rusqlite::Connection::open(&path).unwrap();
        let parent: i64 = db
            .query_row(
                "SELECT id FROM nodes WHERE name=?1",
                ["z".repeat(180)],
                |row| row.get(0),
            )
            .unwrap();
        let service = NativeService::new(path).unwrap();
        let text = service.children_json(snapshot, parent as u64, 0, 100);
        let answer: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(answer["ok"], true, "{answer}");
        assert!(text.len() <= diskgraph_core::QueryBudget::default().max_response_bytes);
        let count = answer["data"]["items"].as_array().unwrap().len();
        assert!(count < 100);
        assert_eq!(answer["data"]["next_offset"], count as u64);
        assert_eq!(answer["data"]["complete"], false);
    }

    #[test]
    fn two_native_handles_for_a_coalesced_job_return_the_same_revision() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        for n in 0..2000 {
            std::fs::write(root.path().join(format!("file-{n}")), b"x").unwrap();
        }
        let service = NativeService::new(
            data.path()
                .join("graph.sqlite")
                .to_string_lossy()
                .into_owned(),
        )
        .unwrap();
        let first = service
            .spawn_scan(root.path().to_string_lossy().into_owned())
            .unwrap();
        let second = service
            .spawn_scan(root.path().to_string_lossy().into_owned())
            .unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "coalesced subscriptions must share last-handle cancellation"
        );
        let left: Value = serde_json::from_str(&first.result_json()).unwrap();
        let right: Value = serde_json::from_str(&second.result_json()).unwrap();
        assert_eq!(left["ok"], true, "{left}");
        assert_eq!(right["ok"], true, "{right}");
        assert_eq!(left["data"]["revision"], right["data"]["revision"]);
    }

    #[test]
    fn completed_native_job_handle_does_not_keep_revoked_permissions() {
        for permission in [
            diskgraph_core::Permission::OperationView,
            diskgraph_core::Permission::MetadataRead,
        ] {
            revoked_job_handle(permission, false);
        }
    }

    #[test]
    fn running_native_job_handle_observes_individual_grant_revocation() {
        for permission in [
            diskgraph_core::Permission::OperationView,
            diskgraph_core::Permission::MetadataRead,
        ] {
            revoked_job_handle(permission, true);
        }
    }

    fn revoked_job_handle(permission: diskgraph_core::Permission, running: bool) {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        if running {
            for n in 0..2000 {
                std::fs::write(root.path().join(format!("file-{n}")), b"x").unwrap();
            }
        }
        let path = data
            .path()
            .join("graph.sqlite")
            .to_string_lossy()
            .into_owned();
        let service = NativeService::new(path).unwrap();
        let handle = service
            .spawn_scan(root.path().to_string_lossy().into_owned())
            .unwrap();
        let principal = local_principal().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let job_id = loop {
            let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
            if let Some(id) = progress["data"]["progress"]["job_id"].as_str() {
                break id.to_owned();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        if running {
            assert!(
                handle.poll_result_json().is_none(),
                "fixture must revoke a running scan"
            );
        } else {
            let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
            assert_eq!(result["ok"], true);
        }
        let job = service.engine.job_status(&job_id).unwrap();
        let scope = job.scope_id;
        let stale_policy = service.engine.policy_authorizer().unwrap();
        let mut control = service.engine.control_store().unwrap();
        control
            .revoke_grant(&principal, &permission, &scope)
            .unwrap();
        drop(control);
        if permission == diskgraph_core::Permission::OperationView {
            assert!(
                service
                    .engine
                    .job_progress(&job_id, &principal, &stale_policy)
                    .is_err(),
                "Engine must intersect stale capability with live grant"
            );
        } else if !running {
            assert!(
                service
                    .engine
                    .revision_for_job(&job_id, &principal, &stale_policy)
                    .is_err()
            );
        }
        let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
        assert_eq!(progress["ok"], false);
        let result: Value = serde_json::from_str(&handle.result_json()).unwrap();
        assert_eq!(
            result["ok"], false,
            "cached result bypassed current permissions: {result}"
        );
        let progress: Value = serde_json::from_str(&handle.progress_json()).unwrap();
        assert_eq!(progress["ok"], false);
        let polled: Value = serde_json::from_str(&handle.poll_result_json().unwrap()).unwrap();
        assert_eq!(polled["ok"], false);
        while !matches!(
            service.engine.job_status(&job_id).unwrap().state,
            diskgraph_store::JobState::Completed
                | diskgraph_store::JobState::Cancelled
                | diskgraph_store::JobState::Failed
        ) {
            assert!(
                std::time::Instant::now() < deadline,
                "background owner must terminate before fixture cleanup"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn last_job_handle_drop_requests_cancel_and_poll_never_joins() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handle = JobHandle {
            cancel: flag.clone(),
            state: std::sync::Arc::new(std::sync::Mutex::new(JobState {
                finished: false,
                result: None,
                progress: None,
                authorization: None,
            })),
        };
        assert!(handle.poll_result_json().is_none());
        drop(handle);
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }
}
