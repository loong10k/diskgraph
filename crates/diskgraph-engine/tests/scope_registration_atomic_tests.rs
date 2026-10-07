//! 注册控制事务故障回归；所有触发器仅在隔离数据库，未执行原生扫描。
use diskgraph_core::{Permission, PrincipalId};
use diskgraph_engine::{Engine, EngineConfig};
use rusqlite::Connection;

#[test]
fn a_failed_default_grant_rolls_back_scope_and_every_grant() {
    for failure in [Permission::MetadataRead, Permission::OperationView] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let data = directory.path().join("data");
        let engine = Engine::open(EngineConfig {
            data_dir: data.clone(),
            ..EngineConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new("atomic-registrar").unwrap();
        engine.bootstrap_local_admin(&actor).unwrap();
        let connection = Connection::open(data.join("diskgraph-control.sqlite")).unwrap();
        connection.execute_batch(&format!(
            "CREATE TRIGGER reject_grant BEFORE INSERT ON grants WHEN NEW.scope_id <> 'diskgraph-admin' AND NEW.permission='{}' BEGIN SELECT RAISE(ABORT,'registration grant fault'); END;", failure.wire_name()
        )).unwrap();
        let result = engine.register_scope(&root, &actor, &engine.policy_authorizer().unwrap());
        assert!(
            result.is_err(),
            "injected grant failure was not observed: {result:?}"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM scopes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{failure:?} retained a partial scope"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM grants WHERE scope_id <> 'diskgraph-admin'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "{failure:?} retained partial grants"
        );
        connection
            .execute_batch("DROP TRIGGER reject_grant")
            .unwrap();
        let scope = engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        assert_eq!(
            scope,
            engine
                .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
                .unwrap()
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM scopes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM grants WHERE scope_id=?1",
                    [scope.as_str()],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            3
        );
    }
}

#[cfg(unix)]
#[test]
fn failed_graph_isolation_cannot_leave_a_new_registered_scope() {
    use diskgraph_core::Locator;
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
    let actor = PrincipalId::new("graph-failure-registrar").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    // 可信旧控制导入制造有损别名组；不要求 macOS 创建非法字节目录。
    engine
        .control_store()
        .unwrap()
        .register_scope(&Locator::from_native_path(&alias), None)
        .unwrap();
    Connection::open(data.join("diskgraph.sqlite"))
        .unwrap()
        .execute_batch(
            "DROP VIEW revision_authorized_ownership; DROP TABLE revision_access_denials;",
        )
        .unwrap();
    let result = engine.register_scope(&root, &actor, &engine.policy_authorizer().unwrap());
    assert!(result.is_err(), "graph fault was not observed");
    let control = engine.control_store().unwrap();
    assert_eq!(
        control.list_scopes().unwrap().len(),
        1,
        "graph failure committed the new scope"
    );
}

#[test]
fn a_deferred_commit_failure_rolls_back_scope_and_grants() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let data = directory.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir: data.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("commit-failure-registrar").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let connection = Connection::open(data.join("diskgraph-control.sqlite")).unwrap();
    connection.execute_batch("CREATE TABLE registration_fault_parent(id INTEGER PRIMARY KEY); CREATE TABLE registration_fault_child(parent_id INTEGER REFERENCES registration_fault_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER reject_commit AFTER INSERT ON scopes BEGIN INSERT INTO registration_fault_child(parent_id) VALUES (1); END;").unwrap();
    assert!(
        engine
            .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
            .is_err()
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM scopes", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM grants WHERE scope_id <> 'diskgraph-admin'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM registration_fault_child", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
}
