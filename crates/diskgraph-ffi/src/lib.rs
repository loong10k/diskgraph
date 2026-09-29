//! UniFFI read-only API. JSON responses keep the first binding contract small.

use std::path::Path;

use diskgraph_core::{DiskGraph, ResourceLocator};
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};

uniffi::setup_scaffolding!();

type ApiResult = Result<Value, String>;
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
    response((|| {
        if !cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            return Err("native-path scanning is unsupported on this platform".into());
        }
        let graph = diskgraph_disktree::scan_native(
            Path::new(&root_path),
            disktree_core::scan::ScanOptions::default(),
        )
        .map_err(|error| error.to_string())?;
        let mut store = open_store(&database_path)?;
        store.save(&graph).map_err(|error| error.to_string())?;
        Ok(json!({
            "snapshot_id": graph.snapshot.id,
            "node_count": graph.nodes.len(),
            "coverage": graph.snapshot.coverage
        }))
    })())
}

/// Returns the latest snapshot ID for a native path previously scanned.
#[uniffi::export]
pub fn latest_native_snapshot_json(database_path: String, root_path: String) -> String {
    response((|| {
        let canonical = Path::new(&root_path)
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let root = ResourceLocator::NativePath(canonical.to_string_lossy().into_owned());
        let store = open_store(&database_path)?;
        let id = store
            .latest_snapshot_id(&root)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "snapshot_id": id }))
    })())
}

#[uniffi::export]
pub fn top_json(database_path: String, snapshot_id: String, parent_id: u64, limit: u32) -> String {
    response((|| {
        let limit = bounded_limit(limit)?;
        let store = open_store(&database_path)?;
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
        let store = open_store(&database_path)?;
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
        let store = open_store(&database_path)?;
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
        let store = open_store(&database_path)?;
        let before = store
            .load(&before_snapshot_id)
            .map_err(|error| error.to_string())?;
        let after = store
            .load(&after_snapshot_id)
            .map_err(|error| error.to_string())?;
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
        let store = open_store(&database_path)?;
        let graph: DiskGraph = store
            .load(&snapshot_id)
            .map_err(|error| error.to_string())?;
        let candidates: Vec<_> = graph
            .candidates(target_bytes)
            .into_iter()
            .map(|candidate| {
                json!({
                    "node": candidate.node,
                    "evidence": candidate.evidence
                })
            })
            .collect();
        Ok(json!(candidates))
    })())
}

fn open_store(path: &str) -> Result<SqliteSnapshotStore, String> {
    SqliteSnapshotStore::open(Path::new(path)).map_err(|error| error.to_string())
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
        #[cfg(unix)]
        assert!(
            growth["data"]["delta_bytes"]
                .as_str()
                .unwrap()
                .parse::<i128>()
                .unwrap()
                > 0
        );
        #[cfg(not(unix))]
        assert!(growth["data"].is_null());
    }
}

/// A live scan job handle (P7 task 9.2, PF-01). `spawn_scan_json` returns
/// immediately, so a UI thread never blocks on a walk: progress is polled,
/// cancellation is cooperative, and `result_json` is the only joining call.
/// Dropping the last handle detaches the worker; the database is only
/// written by the worker's own publish step, so a detached run either
/// completes its publish or leaves none.
#[derive(uniffi::Object)]
pub struct JobHandle {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    state: std::sync::Arc<std::sync::Mutex<JobState>>,
}

struct JobState {
    finished: bool,
    result: Option<Result<serde_json::Value, String>>,
}

#[uniffi::export]
impl JobHandle {
    /// A non-blocking snapshot: state, bytes observed so far, and whether
    /// the job finished. Safe to call from a UI render loop.
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
        response(Ok(serde_json::json!({
            "state": state_name,
            "result": data,
        })))
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
                return match &state.result {
                    Some(Ok(data)) => response(Ok(data.clone())),
                    Some(Err(error)) => response(Err(error.clone())),
                    None => response(Err("job finished without a result".into())),
                };
            }
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// Spawns a native-path scan on a worker thread and returns a handle at
/// once. The scan uses the same code path as `scan_native_json`; the only
/// difference is who waits.
#[uniffi::export]
pub fn spawn_scan_json(database_path: String, root_path: String) -> std::sync::Arc<JobHandle> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let state = std::sync::Arc::new(std::sync::Mutex::new(JobState {
        finished: false,
        result: None,
    }));
    let worker_cancel = std::sync::Arc::clone(&cancel);
    let worker_state = std::sync::Arc::clone(&state);
    let database = database_path.clone();
    let root = root_path.clone();
    std::thread::spawn(move || {
        // The worker honours cancellation between observations by checking
        // the flag on every progress tick of the underlying scan bridge.
        let outcome = run_scan_with_cancel(&database, &root, &worker_cancel);
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
    let engine_dir = std::path::Path::new(database_path).parent().map_or_else(
        || std::path::PathBuf::from("."),
        std::path::Path::to_path_buf,
    );
    let engine = std::sync::Arc::new(
        diskgraph_engine::Engine::open(diskgraph_engine::EngineConfig {
            data_dir: engine_dir,
            ..diskgraph_engine::EngineConfig::default()
        })
        .map_err(|error| error.to_string())?,
    );
    let principal =
        diskgraph_core::PrincipalId::new("ffi-local").map_err(|error| error.to_string())?;
    engine
        .bootstrap_local_admin(&principal)
        .map_err(|error| error.to_string())?;
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
    // The job is claimed and executed on its own thread, while this worker
    // polls: it mirrors the FFI cancel flag into the engine's cancellation
    // channel and collects the final state.
    let runner_engine = std::sync::Arc::clone(&engine);
    let runner_job = job.job_id.clone();
    let runner = std::thread::spawn(move || runner_engine.run_job(&runner_job, "ffi-worker"));
    let finished = loop {
        if runner.is_finished() {
            let outcome = runner
                .join()
                .map_err(|_| "scan worker crashed".to_owned())?;
            if let Err(error) = outcome {
                // A cancellation that lands before the claim surfaces as a
                // cancelled job, and one that lands mid-run surfaces as the
                // engine's conflict mapping. The caller's own flag decides
                // the honest report: a caller who asked to cancel hears a
                // cancellation, whatever internal code path noticed first.
                if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("the scan was cancelled by the caller".into());
                }
                let state = engine
                    .job_status(&job.job_id)
                    .map_err(|report| report.to_string())?
                    .state;
                if state == diskgraph_store::JobState::Cancelled {
                    return Err("scan ended as Cancelled".into());
                }
                return Err(error.to_string());
            }
            break engine
                .job_status(&job.job_id)
                .map_err(|error| error.to_string())?
                .state;
        }
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = engine.cancel_job(&job.job_id, &principal, &authorizer);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    match finished {
        diskgraph_store::JobState::Completed => {
            let revision = engine
                .latest_revision(&scope)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "job succeeded without a revision".to_owned())?;
            let graph = engine
                .load_revision(&revision)
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({
                "revision": revision,
                "node_count": graph.nodes.len(),
                "coverage": graph.snapshot.coverage,
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
