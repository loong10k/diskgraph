use diskgraph_core::ResourceLocator;
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
        serde_json::from_str(&scan_native_json(database_path.clone(), root_path.clone())).unwrap();
    assert_eq!(scanned["ok"], true);
    let snapshot_id = scanned["data"]["snapshot_id"].as_str().unwrap();
    let top: Value =
        serde_json::from_str(&top_json(database_path.clone(), snapshot_id.into(), 1, 10)).unwrap();
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
        serde_json::from_str(&explain_json(database_path.clone(), snapshot_id.into(), 2)).unwrap();
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
        serde_json::from_str(&scan_native_json(database_path.clone(), root_path.clone())).unwrap();
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
