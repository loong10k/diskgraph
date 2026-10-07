//! 旧归属回填只使用可证明无损的身份；所有数据库均为隔离夹具，不启动扫描。
use diskgraph_core::{Locator, ResourceLocator};
use diskgraph_engine::{Engine, EngineConfig};
use diskgraph_store::{ControlStore, SqliteSnapshotStore};

#[path = "support/legacy_graph.rs"]
mod legacy_graph;
use legacy_graph::legacy_graph;

fn backfill(roots: &[Locator], legacy_root: ResourceLocator) -> Option<(String, String)> {
    legacy_database(roots, legacy_root, false, None).0
}

fn legacy_database(
    roots: &[Locator],
    legacy_root: ResourceLocator,
    already_bound: bool,
    native_proof: Option<usize>,
) -> (Option<(String, String)>, bool) {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let mut control = ControlStore::open(&data.join("diskgraph-control.sqlite")).unwrap();
    let server = control.ensure_server().unwrap();
    let mut scopes = Vec::new();
    for root in roots {
        scopes.push(control.register_scope(root, None).unwrap());
    }
    drop(control);
    let graph_path = data.join("diskgraph.sqlite");
    let mut store = SqliteSnapshotStore::open(&graph_path).unwrap();
    let graph = legacy_graph(legacy_root);
    store
        .publish_revision("legacy-job", &graph, "legacy-revision", 1)
        .unwrap();
    drop(store);
    if already_bound {
        // 模拟旧版本已经回填的实际归属行；不借新发布流程制造历史状态。
        rusqlite::Connection::open(&graph_path)
            .unwrap()
            .execute(
                "INSERT INTO revision_ownership(revision_id,server_id,scope_id) VALUES (?1,?2,?3)",
                rusqlite::params!["legacy-revision", server.as_str(), scopes[0].as_str()],
            )
            .unwrap();
    }
    if let Some(proof_scope) = native_proof {
        let locator = diskgraph_core::QualifiedLocator::from_native_path(
            &roots[proof_scope].to_native_path().unwrap(),
        )
        .unwrap();
        rusqlite::Connection::open(&graph_path).unwrap().execute(
            "UPDATE nodes SET native_locator_kind='native_path',native_locator_encoding=?1,native_locator_raw=?2 WHERE snapshot_id='legacy-snapshot' AND id=1",
            rusqlite::params![locator.encoding.wire_name(), locator.raw],
        ).unwrap();
    }
    // 真实 v14 原表升级：旧库没有拒绝表/视图，默认 Engine 必须先一致性备份再迁移。
    rusqlite::Connection::open(&graph_path).unwrap().execute_batch(
        "DROP VIEW revision_authorized_ownership; DROP TABLE revision_access_denials; PRAGMA user_version=14;"
    ).unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let backup = std::fs::read_dir(data.join("migration_backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let backup =
        rusqlite::Connection::open_with_flags(backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM revision_ownership", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        i64::from(already_bound)
    );
    let result = SqliteSnapshotStore::open(&graph_path)
        .unwrap()
        .revision_ownership_for_audit("legacy-revision")
        .unwrap();
    let principal = diskgraph_core::PrincipalId::new("legacy-admin").unwrap();
    engine.bootstrap_local_admin(&principal).unwrap();
    let mut control = engine.control_store().unwrap();
    let policy_version = control.policy_version().unwrap();
    control
        .upsert_grant(&diskgraph_core::Grant {
            principal: principal.clone(),
            permission: diskgraph_core::Permission::MetadataRead,
            scope: scopes[0].clone(),
            policy_version,
        })
        .unwrap();
    drop(control);

    let authorized = engine
        .authorize_revision(
            None,
            "legacy-revision",
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .is_ok();
    drop(engine);
    (result, authorized)
}

#[test]
fn exact_unicode_native_root_still_backfills() {
    let path = std::path::Path::new("/isolated/目录");
    assert!(
        backfill(
            &[Locator::from_native_path(path)],
            ResourceLocator::NativePath(path.to_str().unwrap().into())
        )
        .is_some()
    );
}

#[cfg(unix)]
#[test]
fn single_non_utf8_display_alias_must_not_claim_legacy_revision() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/isolated/r\xff".to_vec()));
    let root = Locator::from_native_path(&path);
    assert!(
        backfill(
            std::slice::from_ref(&root),
            ResourceLocator::NativePath(root.display.clone())
        )
        .is_none(),
        "unique display match is not a lossless scope identity"
    );
}

#[cfg(unix)]
#[test]
fn excluding_lossy_scope_must_not_turn_colliding_unicode_scope_into_unique_match() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/isolated/r\xff".to_vec()));
    let lossy = Locator::from_native_path(&path);
    let unicode = Locator::from_native_path(std::path::Path::new(&lossy.display));
    assert!(
        backfill(
            &[unicode, lossy.clone()],
            ResourceLocator::NativePath(lossy.display)
        )
        .is_none()
    );
}

#[cfg(windows)]
#[test]
fn single_unpaired_utf16_display_alias_must_not_claim_legacy_revision() {
    use std::os::windows::ffi::OsStringExt;
    let units = [u16::from(b'C'), u16::from(b':'), u16::from(b'\\'), 0xd800];
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&units));
    let root = Locator::from_native_path(&path);
    assert!(
        backfill(
            std::slice::from_ref(&root),
            ResourceLocator::NativePath(root.display.clone())
        )
        .is_none()
    );
}

#[cfg(unix)]
#[test]
fn historical_lossy_binding_is_denied_without_erasing_original_ownership() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/isolated/r\xff".to_vec()));
    let root = Locator::from_native_path(&path);
    let (original, authorized) = legacy_database(
        std::slice::from_ref(&root),
        ResourceLocator::NativePath(root.display.clone()),
        true,
        None,
    );
    assert!(
        original.is_some(),
        "audit must preserve original historical mapping"
    );
    assert!(
        !authorized,
        "persisted display-only historical ownership must not grant access"
    );
}

#[cfg(unix)]
#[test]
fn actual_persisted_native_root_proof_preserves_non_utf8_scope_access() {
    use std::os::unix::ffi::OsStringExt;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/isolated/r\xff".to_vec()));
    let root = Locator::from_native_path(&path);
    let (original, authorized) = legacy_database(
        std::slice::from_ref(&root),
        ResourceLocator::NativePath(root.display.clone()),
        true,
        Some(0),
    );
    assert!(original.is_some());
    assert!(
        authorized,
        "exact original native root was incorrectly isolated"
    );
}

#[test]
fn replacing_authorized_view_cannot_enable_a_new_engine() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    drop(
        Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    rusqlite::Connection::open(data.join("diskgraph.sqlite")).unwrap().execute_batch(
        "DROP VIEW revision_authorized_ownership; CREATE VIEW revision_authorized_ownership AS SELECT * FROM revision_ownership;"
    ).unwrap();
    assert!(
        Engine::open(EngineConfig {
            data_dir: data,
            ..EngineConfig::default()
        })
        .is_err()
    );
}

#[test]
fn failed_v15_migration_preserves_exact_v14_backup_and_does_not_enable_engine() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    drop(
        Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap(),
    );
    let graph = data.join("diskgraph.sqlite");
    rusqlite::Connection::open(&graph).unwrap().execute_batch(
        "DROP VIEW revision_authorized_ownership; DROP TABLE revision_access_denials; PRAGMA user_version=14; CREATE TABLE revision_access_denials(incompatible TEXT);"
    ).unwrap();
    assert!(
        Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .is_err()
    );
    let backup = std::fs::read_dir(data.join("migration_backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    for path in [&graph, &backup] {
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            14
        );
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE type='view' AND name='revision_authorized_ownership'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
}

#[cfg(unix)]
#[test]
fn another_scopes_raw_root_cannot_validate_same_display_historical_owner() {
    use std::os::unix::ffi::OsStringExt;
    let roots = [0xff, 0xfe].map(|byte| {
        let mut raw = b"/isolated/r".to_vec();
        raw.push(byte);
        Locator::from_native_path(&std::path::PathBuf::from(std::ffi::OsString::from_vec(raw)))
    });
    assert_eq!(roots[0].display, roots[1].display);
    assert_ne!(roots[0].raw_b64, roots[1].raw_b64);
    let (original, authorized) = legacy_database(
        &roots,
        ResourceLocator::NativePath(roots[0].display.clone()),
        true,
        Some(1),
    );
    assert!(original.is_some());
    assert!(
        !authorized,
        "different raw root incorrectly confirmed the displayed owner"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn live_scope_registration_isolates_existing_lossy_alias_without_restart() {
    use std::os::unix::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("r�");
    let alias = directory
        .path()
        .join(std::ffi::OsString::from_vec(b"r\xff".to_vec()));
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&alias).unwrap();
    let data = directory.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = diskgraph_core::PrincipalId::new("live-admin").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    drop(engine);
    SqliteSnapshotStore::open(&data.join("diskgraph.sqlite"))
        .unwrap()
        .publish_revision(
            "legacy-job",
            &legacy_graph(ResourceLocator::NativePath(
                root.canonicalize().unwrap().to_str().unwrap().into(),
            )),
            "legacy-revision",
            1,
        )
        .unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    assert!(
        engine
            .authorize_revision(
                Some(&scope),
                "legacy-revision",
                &actor,
                &engine.policy_authorizer().unwrap()
            )
            .is_ok()
    );
    engine
        .register_scope(&alias, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert!(
        engine
            .authorize_revision(
                Some(&scope),
                "legacy-revision",
                &actor,
                &engine.policy_authorizer().unwrap()
            )
            .is_err(),
        "live registration must isolate the alias group before granting access"
    );
    assert!(
        SqliteSnapshotStore::open(&data.join("diskgraph.sqlite"))
            .unwrap()
            .revision_ownership_for_audit("legacy-revision")
            .unwrap()
            .is_some()
    );
}

#[test]
#[cfg(unix)]
fn ordinary_registration_reconciles_live_legacy_alias_and_repeat_cannot_unquarantine() {
    use std::os::unix::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("r�");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let alias = root
        .parent()
        .unwrap()
        .join(std::ffi::OsString::from_vec(b"r\xff".to_vec()));
    let data = directory.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = diskgraph_core::PrincipalId::new("live-legacy-admin").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    // 模拟运行中可信旧库导入；不声称本机文件系统允许创建无效字节目录。
    let (server, alias_scope) = {
        let mut control = engine.control_store().unwrap();
        let server = control.ensure_server().unwrap();
        let scope = control
            .register_scope(&Locator::from_native_path(&alias), None)
            .unwrap();
        let policy_version = control.policy_version().unwrap();
        control
            .upsert_grant(&diskgraph_core::Grant {
                principal: actor.clone(),
                permission: diskgraph_core::Permission::MetadataRead,
                scope: scope.clone(),
                policy_version,
            })
            .unwrap();
        (server, scope)
    };
    let graph_path = data.join("diskgraph.sqlite");
    SqliteSnapshotStore::open(&graph_path)
        .unwrap()
        .publish_revision(
            "legacy-job",
            &legacy_graph(ResourceLocator::NativePath(root.to_str().unwrap().into())),
            "legacy-revision",
            1,
        )
        .unwrap();
    rusqlite::Connection::open(&graph_path)
        .unwrap()
        .execute(
            "INSERT INTO revision_ownership(revision_id,server_id,scope_id) VALUES (?1,?2,?3)",
            rusqlite::params!["legacy-revision", server.as_str(), alias_scope.as_str()],
        )
        .unwrap();
    assert!(
        engine
            .authorize_revision(
                Some(&alias_scope),
                "legacy-revision",
                &actor,
                &engine.policy_authorizer().unwrap()
            )
            .is_ok()
    );
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert!(
        engine
            .authorize_revision(
                Some(&alias_scope),
                "legacy-revision",
                &actor,
                &engine.policy_authorizer().unwrap()
            )
            .is_err()
    );
    assert_eq!(
        scope,
        engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap()
    );
    assert!(
        engine
            .authorize_revision(
                Some(&alias_scope),
                "legacy-revision",
                &actor,
                &engine.policy_authorizer().unwrap()
            )
            .is_err()
    );
    assert_eq!(
        SqliteSnapshotStore::open(&graph_path)
            .unwrap()
            .revision_ownership_for_audit("legacy-revision")
            .unwrap(),
        Some((server.as_str().to_owned(), alias_scope.as_str().to_owned()))
    );
}
