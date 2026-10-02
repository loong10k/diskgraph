use crate::{ControlStore, JobKind, JobState, StoreError};
use diskgraph_core::{Grant, Locator, LocatorKind, Permission, PrincipalId, ScopeId};

#[test]
fn a_repeated_grant_does_not_require_a_write_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    let mut store = crate::ControlStore::open(&path).unwrap();
    let grant = diskgraph_core::Grant {
        principal: diskgraph_core::PrincipalId::new("local").unwrap(),
        permission: diskgraph_core::Permission::MetadataRead,
        scope: diskgraph_core::ScopeId::new("fixture").unwrap(),
        policy_version: 1,
    };
    store.upsert_grant(&grant).unwrap();
    let observer = rusqlite::Connection::open(&path).unwrap();
    observer
        .execute_batch("BEGIN; SELECT * FROM grants;")
        .unwrap();
    store
        .connection
        .busy_timeout(std::time::Duration::from_millis(20))
        .unwrap();
    let changed = store.connection.total_changes();
    // 活跃共享读事务不妨碍幂等 bootstrap；旧 REPLACE 会等待独占提交并失败。
    store.upsert_grant(&grant).unwrap();
    assert_eq!(store.connection.total_changes(), changed);
    observer.execute_batch("ROLLBACK;").unwrap();
}

use diskgraph_core::Authorizer as _;

fn native_root(name: &str) -> Locator {
    Locator {
        kind: LocatorKind::NativePath,
        raw_b64: name.to_owned(),
        display: format!("/display/{name}"),
    }
}

fn scope(name: &str) -> ScopeId {
    ScopeId::new(name).unwrap()
}

fn principal(name: &str) -> PrincipalId {
    PrincipalId::new(name).unwrap()
}

#[test]
fn server_identity_is_minted_once_and_persisted() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("control.sqlite");
    let first = {
        let mut store = ControlStore::open(&db).unwrap();
        store.ensure_server().unwrap()
    };
    let second = {
        let mut store = ControlStore::open(&db).unwrap();
        store.ensure_server().unwrap()
    };
    assert_eq!(first, second);
}

#[cfg(unix)]
#[test]
fn control_database_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("control.sqlite");
    ControlStore::open(&db).unwrap();
    let mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "control records must not be world-readable");
}

#[test]
fn scope_registration_is_idempotent_until_revoked() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let first = store
        .register_scope(&native_root("one"), Some("vol-1"))
        .unwrap();
    let second = store
        .register_scope(&native_root("one"), Some("vol-1"))
        .unwrap();
    assert_eq!(first, second);

    store.revoke_scope(&first).unwrap();
    assert!(store.scope(&first).unwrap().revoked);
    assert!(matches!(
        store.register_scope(&native_root("one"), None),
        Err(StoreError::Conflict(_))
    ));

    let other = store.register_scope(&native_root("two"), None).unwrap();
    assert_ne!(first, other);
    assert_eq!(store.list_scopes().unwrap().len(), 2);
}

#[test]
fn revoked_scopes_reject_new_jobs_but_records_survive() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope_id = store.register_scope(&native_root("project"), None).unwrap();
    let job = store
        .create_job(&scope_id, JobKind::Index, &principal("agent"))
        .unwrap();
    store.revoke_scope(&scope_id).unwrap();
    assert!(matches!(
        store.create_job(&scope_id, JobKind::Sync, &principal("agent")),
        Err(StoreError::Conflict(_))
    ));
    // Revocation never erases history: the old job stays queryable.
    assert_eq!(store.job(&job.job_id).unwrap().scope_id, scope_id);
    // Revoking again is idempotent.
    store.revoke_scope(&scope_id).unwrap();
}

#[test]
fn cancellation_is_durable_and_invalidates_the_running_publish_fence() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope_id = store.register_scope(&native_root("cancel"), None).unwrap();
    let job = store
        .create_job(&scope_id, JobKind::Index, &principal("agent"))
        .unwrap();
    let claimed = store.claim_job_once(&job.job_id, "worker").unwrap();
    assert!(store.request_cancel(&job.job_id).unwrap());
    assert!(
        store
            .cancellation_requested(&job.job_id, claimed.fencing_token)
            .unwrap()
    );
    assert!(matches!(
        store.with_job_fence(&job.job_id, "worker", claimed.fencing_token, || Ok(())),
        Err(StoreError::StaleOwner)
    ));
    let terminal = store
        .finish_job_fenced(
            &job.job_id,
            "worker",
            claimed.fencing_token,
            JobState::Cancelled,
        )
        .unwrap();
    assert_eq!(terminal.state, JobState::Cancelled);
}

#[test]
fn v4_jobs_migrate_to_persistent_cancellation_with_backup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    let mut first = ControlStore::open(&path).unwrap();
    let scope_id = first
        .register_scope(&native_root("migration"), None)
        .unwrap();
    let job = first
        .create_job(&scope_id, JobKind::Index, &principal("agent"))
        .unwrap();
    drop(first);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch("ALTER TABLE jobs DROP COLUMN cancel_requested; PRAGMA user_version = 4;")
        .unwrap();
    drop(connection);
    let mut migrated = ControlStore::open(&path).unwrap();
    assert_eq!(migrated.job(&job.job_id).unwrap().state, JobState::Queued);
    assert!(migrated.request_cancel(&job.job_id).unwrap());
    assert_eq!(
        migrated.job(&job.job_id).unwrap().state,
        JobState::Cancelled
    );
    assert!(
        directory
            .path()
            .join("migration_backups/control.sqlite.pre-v5.bak")
            .is_file()
    );
}

#[test]
fn revoked_expired_job_does_not_precede_eligible_work_forever() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let revoked = store.register_scope(&native_root("revoked"), None).unwrap();
    let eligible = store
        .register_scope(&native_root("eligible"), None)
        .unwrap();
    let first = store
        .create_job(&revoked, JobKind::Index, &principal("agent"))
        .unwrap();
    let claimed = store.claim_job_once(&first.job_id, "dead-worker").unwrap();
    store
        .with_connection(|connection| {
            connection.execute(
                "UPDATE jobs SET lease_expires_unix_ms = 0 WHERE job_id = ?1",
                [&first.job_id],
            )?;
            Ok(())
        })
        .unwrap();
    store.revoke_scope(&revoked).unwrap();
    let second = store
        .create_job(&eligible, JobKind::Index, &principal("agent"))
        .unwrap();
    let candidates = store.list_queued_jobs().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].job_id, second.job_id);
    assert_eq!(store.job(&first.job_id).unwrap().state, JobState::Cancelled);
    assert_eq!(claimed.state, JobState::Running);
}

#[test]
fn policy_grants_round_trip_through_storage_and_rebuild_the_authorizer() {
    let mut store = ControlStore::open_in_memory().unwrap();
    store.publish_policy_version(3).unwrap();
    store
        .upsert_grant(&Grant {
            principal: principal("agent"),
            permission: Permission::MetadataRead,
            scope: scope("project"),
            policy_version: 3,
        })
        .unwrap();
    // Older-version grant recorded underneath the current version.
    store
        .upsert_grant(&Grant {
            principal: principal("agent"),
            permission: Permission::ContentRead,
            scope: scope("project"),
            policy_version: 2,
        })
        .unwrap();

    let authorizer = store.authorizer().unwrap();
    assert!(matches!(
        authorizer.decide(
            &principal("agent"),
            &Permission::MetadataRead,
            &scope("project")
        ),
        diskgraph_core::Decision::Allowed
    ));
    assert!(matches!(
        authorizer.decide(
            &principal("agent"),
            &Permission::ContentRead,
            &scope("project")
        ),
        diskgraph_core::Decision::Denied(diskgraph_core::DenyReason::PolicyVersionMismatch)
    ));

    store.revoke_policy().unwrap();
    let authorizer = store.authorizer().unwrap();
    assert!(matches!(
        authorizer.decide(
            &principal("agent"),
            &Permission::MetadataRead,
            &scope("project")
        ),
        diskgraph_core::Decision::Denied(_)
    ));
}

#[test]
fn jobs_merge_per_scope_and_enforce_owner_fencing() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope_id = store.register_scope(&native_root("project"), None).unwrap();
    let first = store
        .create_job(&scope_id, JobKind::Index, &principal("agent"))
        .unwrap();
    let merged = store
        .create_job(&scope_id, JobKind::Sync, &principal("agent"))
        .unwrap();
    assert_eq!(
        first.job_id, merged.job_id,
        "active jobs must merge per scope"
    );

    let job_id = first.job_id.clone();
    let claimed = store.claim_job(&job_id, "worker-a").unwrap();
    assert_eq!(claimed.state, JobState::Running);
    assert_eq!(claimed.owner, "worker-a");

    // Fencing: another owner can neither claim, heartbeat, nor finish.
    assert!(matches!(
        store.claim_job(&job_id, "worker-b").unwrap_err(),
        StoreError::StaleOwner
    ));
    assert!(matches!(
        store.heartbeat(&job_id, "worker-b").unwrap_err(),
        StoreError::StaleOwner
    ));
    assert!(matches!(
        store
            .finish_job(&job_id, "worker-b", JobState::Completed)
            .unwrap_err(),
        StoreError::StaleOwner
    ));

    store.heartbeat(&job_id, "worker-a").unwrap();
    let finished = store
        .finish_job(&job_id, "worker-a", JobState::Completed)
        .unwrap();
    assert_eq!(finished.state, JobState::Completed);
    assert!(store.active_job_for_scope(&scope_id).unwrap().is_none());
    assert!(matches!(
        store.claim_job(&job_id, "worker-a"),
        Err(StoreError::Conflict(_))
    ));
}

#[test]
fn unknown_scope_and_job_ids_are_reported_not_invented() {
    let store = ControlStore::open_in_memory().unwrap();
    assert!(matches!(
        store.scope(&scope("missing")),
        Err(StoreError::ScopeNotFound(_))
    ));
    assert!(matches!(
        store.job("job-none"),
        Err(StoreError::JobNotFound(_))
    ));
}
