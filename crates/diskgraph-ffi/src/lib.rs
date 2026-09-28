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
