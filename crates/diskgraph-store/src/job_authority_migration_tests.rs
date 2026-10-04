use crate::job_authority_tests::{configure, principal};
use crate::{ControlStore, JobKind, StoreError};

#[test]
fn real_v6_upgrade_keeps_a_consistent_backup_and_unknown_old_jobs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    let job = {
        let mut store = ControlStore::open(&path).unwrap();
        let scope = configure(&mut store);
        let job = store
            .create_job(&scope, JobKind::Index, &principal())
            .unwrap();
        store.connection.execute_batch("DROP TRIGGER job_authority_immutable_update; DROP TRIGGER job_authority_immutable_delete; DROP TABLE job_request_authorities; PRAGMA user_version=6;").unwrap();
        job
    };
    let upgraded = ControlStore::open(&path).unwrap();
    assert_eq!(
        upgraded
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(upgraded.job(&job.job_id).unwrap(), job);
    assert_eq!(upgraded.job_request_authority(&job.job_id).unwrap(), None);
    let backup = rusqlite::Connection::open(
        directory
            .path()
            .join("migration_backups/control.sqlite.pre-v7.bak"),
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        backup
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(
        backup
            .prepare("SELECT * FROM job_request_authorities")
            .is_err()
    );
}

#[test]
fn failed_authority_migration_does_not_enable_v7_or_erase_backup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    {
        let store = ControlStore::open(&path).unwrap();
        store.connection.execute_batch("DROP TRIGGER job_authority_immutable_update; DROP TRIGGER job_authority_immutable_delete; DROP TABLE job_request_authorities; CREATE TABLE job_request_authorities(job_id TEXT PRIMARY KEY,wrong_field TEXT); PRAGMA user_version=6;").unwrap();
    }
    assert!(matches!(
        ControlStore::open(&path),
        Err(StoreError::InvalidGraph(_))
    ));
    for database in [
        path,
        directory
            .path()
            .join("migration_backups/control.sqlite.pre-v7.bak"),
    ] {
        let connection = rusqlite::Connection::open(database).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            6
        );
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'job_authority_immutable_%'",[],|row| row.get::<_,i64>(0)).unwrap(),0);
    }
}
