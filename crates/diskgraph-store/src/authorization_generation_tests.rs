use crate::ControlStore;

fn generation(store: &ControlStore) -> i64 {
    store
        .connection
        .query_row(
            "SELECT generation FROM authorization_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn authorization_generation_tracks_individual_grants_scopes_and_policy() {
    let store = ControlStore::open_in_memory().unwrap();
    let mut previous = generation(&store);
    for sql in [
        "INSERT INTO policy VALUES (1, 1, 0)",
        "INSERT INTO scopes VALUES ('scope', 'unix_bytes', 'AA==', '/fixture', NULL, 1, 0)",
        "INSERT INTO grants VALUES ('alice', 'metadata:read', 'scope', 1)",
        "DELETE FROM grants WHERE principal_id = 'alice'",
        "UPDATE scopes SET revoked = 1 WHERE scope_id = 'scope'",
        "UPDATE policy SET revoked = 1 WHERE id = 1",
    ] {
        store.connection.execute(sql, []).unwrap();
        assert!(generation(&store) > previous, "{sql}");
        previous = generation(&store);
    }
}

#[test]
fn unrelated_job_writes_do_not_invalidate_queued_results() {
    let store = ControlStore::open_in_memory().unwrap();
    store
        .connection
        .execute(
            "INSERT INTO scopes VALUES ('scope', 'unix_bytes', 'AA==', '/fixture', NULL, 1, 0)",
            [],
        )
        .unwrap();
    let before = generation(&store);
    store
        .connection
        .execute("INSERT INTO server VALUES (1, 'fixture', 1)", [])
        .unwrap();
    store.connection.execute("INSERT INTO jobs (job_id, scope_id, kind, state, created_at_unix_ms, heartbeat_unix_ms) VALUES ('job', 'scope', 'index', 'queued', 1, 1)", []).unwrap();
    store
        .connection
        .execute(
            "UPDATE jobs SET heartbeat_unix_ms = 2 WHERE job_id = 'job'",
            [],
        )
        .unwrap();
    assert_eq!(generation(&store), before);
}

#[test]
fn v5_upgrade_has_a_consistent_backup_and_cross_connection_invalidation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    {
        let store = ControlStore::open(&path).unwrap();
        store
            .connection
            .execute("INSERT INTO policy VALUES (1, 1, 0)", [])
            .unwrap();
        for table in ["policy", "grants", "scopes"] {
            for event in ["insert", "update", "delete"] {
                store
                    .connection
                    .execute_batch(&format!(
                        "DROP TRIGGER IF EXISTS auth_generation_{table}_{event};"
                    ))
                    .unwrap();
            }
        }
        store
            .connection
            .execute_batch("DROP TABLE IF EXISTS authorization_state; PRAGMA user_version = 5;")
            .unwrap();
    }
    let store = ControlStore::open(&path).unwrap();
    assert_eq!(
        store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    let backup = rusqlite::Connection::open(
        dir.path()
            .join("migration_backups/control.sqlite.pre-v6.bak"),
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM policy", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let before = generation(&store);
    let second = rusqlite::Connection::open(&path).unwrap();
    second
        .execute(
            "INSERT INTO grants VALUES ('alice', 'metadata:read', 'scope', 1)",
            [],
        )
        .unwrap();
    assert!(generation(&store) > before);
}

#[test]
fn rolled_back_or_duplicate_grants_do_not_change_generation_and_overflow_fails_closed() {
    let store = ControlStore::open_in_memory().unwrap();
    store
        .connection
        .execute(
            "INSERT INTO grants VALUES ('alice', 'metadata:read', 'scope', 1)",
            [],
        )
        .unwrap();
    let before = generation(&store);
    store
        .connection
        .execute(
            "INSERT OR IGNORE INTO grants VALUES ('alice', 'metadata:read', 'scope', 1)",
            [],
        )
        .unwrap();
    assert_eq!(generation(&store), before);
    store
        .connection
        .execute_batch("BEGIN; DELETE FROM grants WHERE principal_id = 'alice';")
        .unwrap();
    assert!(generation(&store) > before);
    store.connection.execute_batch("ROLLBACK;").unwrap();
    assert_eq!(generation(&store), before);
    store
        .connection
        .execute(
            "UPDATE authorization_state SET generation = ?1 WHERE id = 1",
            [i64::MAX],
        )
        .unwrap();
    assert!(
        store
            .connection
            .execute("DELETE FROM grants WHERE principal_id = 'alice'", [])
            .is_err()
    );
    assert_eq!(generation(&store), i64::MAX);
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM grants", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn deadline_reads_restore_the_actual_connection_timeout_and_leave_no_expired_callback() {
    let store = ControlStore::open_in_memory().unwrap();
    let timeout = || {
        store
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    let original = timeout();
    store
        .authorization_generation_until(
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(timeout(), original);
    store
        .connection
        .busy_timeout(std::time::Duration::from_millis(37))
        .unwrap();
    store
        .authorization_generation_until(
            std::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(timeout(), 37);
    assert!(matches!(
        store.authorization_generation_until(std::time::Instant::now()),
        Err(crate::StoreError::BudgetExceeded)
    ));
    assert_eq!(store.authorization_generation().unwrap(), 0);
    assert_eq!(timeout(), 37);
}

#[test]
fn a_failed_v6_migration_keeps_v5_data_and_rolls_back_all_new_objects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    {
        let store = ControlStore::open(&path).unwrap();
        store
            .connection
            .execute("INSERT INTO policy VALUES (1, 7, 0)", [])
            .unwrap();
        for table in ["policy", "grants", "scopes"] {
            for event in ["insert", "update", "delete"] {
                store
                    .connection
                    .execute_batch(&format!("DROP TRIGGER auth_generation_{table}_{event};"))
                    .unwrap();
            }
        }
        store.connection.execute_batch("DROP TABLE authorization_state; PRAGMA user_version = 5; CREATE TRIGGER auth_generation_policy_insert AFTER INSERT ON policy BEGIN SELECT 1; END;").unwrap();
    }
    assert!(ControlStore::open(&path).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(
        db.query_row("SELECT version FROM policy", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='authorization_state'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        dir.path()
            .join("migration_backups/control.sqlite.pre-v6.bak")
            .is_file()
    );
}
