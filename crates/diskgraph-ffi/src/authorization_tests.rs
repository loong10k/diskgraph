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
    let config = diskgraph_engine::EngineConfig {
        data_dir: data.path().to_path_buf(),
        graph_database_path: Some(database.clone()),
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    let (engine, _scan_owner) =
        crate::native_legacy_fixture::NativeLegacyFixture::open(config).unwrap();
    #[cfg(not(target_os = "linux"))]
    let engine = diskgraph_engine::Engine::open(config).unwrap();
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
        let config = diskgraph_engine::EngineConfig {
            data_dir: data.path().to_path_buf(),
            graph_database_path: Some(database.clone()),
            ..Default::default()
        };
        #[cfg(target_os = "linux")]
        let (engine, _scan_owner) =
            crate::native_legacy_fixture::NativeLegacyFixture::open(config).unwrap();
        #[cfg(not(target_os = "linux"))]
        let engine = diskgraph_engine::Engine::open(config).unwrap();
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
        let result: serde_json::Value = serde_json::from_str(&super::latest_native_snapshot_json(
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
            diskgraph_store::ControlStore::open(&realm.join("diskgraph-control.sqlite")).unwrap();
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
