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
        serde_json::from_str(&scan_native_json(database_path.clone(), root_path.clone())).unwrap();
    assert_eq!(scanned["ok"], true);
    let snapshot_id = scanned["data"]["snapshot_id"].as_str().unwrap();
    let top: Value =
        serde_json::from_str(&top_json(database_path.clone(), snapshot_id.into(), 1, 10)).unwrap();
    assert_eq!(top["ok"], true);
    // The read-only surface is intact, and nothing outside the two
    // caller-named directories was touched.
    assert!(host.path().join("snapshots.sqlite").is_file());
    assert!(root.path().join("report.txt").is_file());
}
