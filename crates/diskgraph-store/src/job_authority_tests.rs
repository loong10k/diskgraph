use crate::{ControlStore, JobKind, JobState, StoreError};
use diskgraph_core::{
    Grant, JobRequestAuthority, Locator, LocatorKind, Permission, PrincipalId, ScopeId,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) fn principal() -> PrincipalId {
    PrincipalId::new("alice").unwrap()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn fixture() -> (ControlStore, ScopeId) {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope = configure(&mut store);
    (store, scope)
}

pub(super) fn configure(store: &mut ControlStore) -> ScopeId {
    let scope = store
        .register_scope(
            &Locator {
                kind: LocatorKind::NativePath,
                raw_b64: "L2ZpeHR1cmU=".into(),
                display: "/fixture".into(),
            },
            None,
        )
        .unwrap();
    store.publish_policy_version(1).unwrap();
    store
        .upsert_grant(&Grant {
            principal: principal(),
            permission: Permission::IndexWrite,
            scope: scope.clone(),
            policy_version: 1,
        })
        .unwrap();
    scope
}

fn remote(expiry: u64, permissions: Vec<Permission>) -> JobRequestAuthority {
    JobRequestAuthority::authenticated_remote(
        principal(),
        "actual-issuer",
        "http",
        permissions,
        expiry,
    )
    .unwrap()
}

fn wait_until_expired(expiry: u64) {
    let end = Instant::now() + Duration::from_secs(4);
    while now() < expiry {
        assert!(
            Instant::now() < end,
            "bounded fixture clock failed to reach expiry"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn persisted_authority_reopens_without_bearer_or_job_record_shape_change() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sqlite");
    let authority = remote(now() + 60, vec![Permission::IndexWrite]);
    let job = {
        let mut store = ControlStore::open(&path).unwrap();
        let scope = configure(&mut store);
        store
            .create_job_with_authority(&scope, JobKind::Index, &authority, 16)
            .unwrap()
            .unwrap()
    };
    let reopened = ControlStore::open(&path).unwrap();
    assert_eq!(
        reopened.job_request_authority(&job.job_id).unwrap(),
        Some(authority)
    );
    assert_eq!(reopened.job(&job.job_id).unwrap(), job);
    let json = serde_json::to_value(&job).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 10);
    assert!(json.get("authority").is_none());
    let persisted: String = reopened
        .connection
        .query_row(
            "SELECT authority_json FROM job_request_authorities WHERE job_id=?1",
            [&job.job_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!persisted.contains("bearer"));
    assert_eq!(
        serde_json::from_str::<JobRequestAuthority>(&persisted)
            .unwrap()
            .principal(),
        &principal()
    );
}

#[test]
fn remote_merges_only_same_kind_and_exact_normalized_original_authority() {
    let (mut store, scope) = fixture();
    let expiry = now() + 60;
    let first_authority = remote(
        expiry,
        vec![Permission::IndexWrite, Permission::MetadataRead],
    );
    let first = store
        .create_job_with_authority(&scope, JobKind::Index, &first_authority, 16)
        .unwrap()
        .unwrap();
    let normalized = remote(
        expiry,
        vec![
            Permission::MetadataRead,
            Permission::IndexWrite,
            Permission::IndexWrite,
        ],
    );
    let same = store
        .create_job_with_authority(&scope, JobKind::Index, &normalized, 16)
        .unwrap()
        .unwrap();
    assert_eq!(same.job_id, first.job_id);
    for (kind, authority) in [
        (JobKind::Sync, normalized),
        (
            JobKind::Index,
            remote(
                expiry + 1,
                vec![Permission::IndexWrite, Permission::MetadataRead],
            ),
        ),
        (JobKind::Index, remote(expiry, vec![Permission::IndexWrite])),
    ] {
        assert_ne!(
            store
                .create_job_with_authority(&scope, kind, &authority, 16)
                .unwrap()
                .unwrap()
                .job_id,
            first.job_id
        );
    }
    let legacy = store
        .create_job(&scope, JobKind::Index, &principal())
        .unwrap();
    assert_ne!(
        legacy.job_id, first.job_id,
        "trusted legacy cannot merge into remote authority"
    );
    assert_eq!(
        store.active_job_count_for_principal(&principal()).unwrap(),
        5
    );
    assert!(
        store
            .create_job_with_authority(
                &scope,
                JobKind::Sync,
                &remote(expiry + 2, vec![Permission::IndexWrite]),
                5
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn explicit_local_keeps_cross_kind_merge_and_no_policy_compatibility() {
    let (mut store, scope) = fixture();
    store.connection.execute("DELETE FROM policy", []).unwrap();
    let local = JobRequestAuthority::trusted_local(principal(), "trusted-engine").unwrap();
    let index = store
        .create_job_with_authority(&scope, JobKind::Index, &local, 16)
        .unwrap()
        .unwrap();
    let sync = store
        .create_job_with_authority(&scope, JobKind::Sync, &local, 16)
        .unwrap()
        .unwrap();
    assert_eq!(index.job_id, sync.job_id);
    assert!(matches!(
        store.create_job_with_authority(
            &scope,
            JobKind::Index,
            &remote(now() + 60, vec![Permission::IndexWrite]),
            16
        ),
        Err(StoreError::Conflict(_))
    ));
    assert!(
        store
            .claim_job_once_strict(&index.job_id, "local-worker")
            .is_ok()
    );
}

#[test]
fn capability_ceiling_and_live_grants_are_both_required_to_enqueue() {
    let (mut store, scope) = fixture();
    assert!(matches!(
        store.create_job_with_authority(
            &scope,
            JobKind::Index,
            &remote(now() + 60, vec![Permission::MetadataRead]),
            16
        ),
        Err(StoreError::Conflict(_))
    ));
    store
        .revoke_grant(&principal(), &Permission::IndexWrite, &scope)
        .unwrap();
    assert!(matches!(
        store.create_job_with_authority(
            &scope,
            JobKind::Index,
            &remote(now() + 60, vec![Permission::IndexWrite]),
            16
        ),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(
        store.active_job_count_for_principal(&principal()).unwrap(),
        0
    );
}

#[test]
fn queued_expiry_is_failed_by_strict_and_trusted_claim_without_delegation() {
    let (mut store, scope) = fixture();
    let expiry = now() + 2;
    let authority = remote(expiry, vec![Permission::IndexWrite]);
    let strict = store
        .create_job_with_authority(&scope, JobKind::Index, &authority, 16)
        .unwrap()
        .unwrap();
    let trusted = store
        .create_job_with_authority(&scope, JobKind::Sync, &authority, 16)
        .unwrap()
        .unwrap();
    wait_until_expired(expiry);
    assert!(matches!(
        store.claim_job_once_strict(&strict.job_id, "strict"),
        Err(StoreError::Conflict(_))
    ));
    assert!(matches!(
        store.claim_job_once(&trusted.job_id, "trusted"),
        Err(StoreError::Conflict(_))
    ));
    for id in [&strict.job_id, &trusted.job_id] {
        let record = store.job(id).unwrap();
        assert_eq!(record.state, JobState::Failed);
        assert_eq!(record.fencing_token, 0);
    }
}

#[test]
fn running_expiry_rejects_heartbeat_and_old_fence_but_failure_can_be_persisted() {
    let (mut store, scope) = fixture();
    let expiry = now() + 2;
    let authority = remote(expiry, vec![Permission::IndexWrite]);
    let queued = store
        .create_job_with_authority(&scope, JobKind::Index, &authority, 16)
        .unwrap()
        .unwrap();
    let claimed = store
        .claim_job_once_strict(&queued.job_id, "worker")
        .unwrap();
    wait_until_expired(expiry);
    assert!(matches!(
        store.heartbeat_fenced(&claimed.job_id, "worker", claimed.fencing_token),
        Err(StoreError::Conflict(_))
    ));
    let mut ran = false;
    assert!(matches!(
        store.with_job_fence(&claimed.job_id, "worker", claimed.fencing_token, || {
            ran = true;
            Ok(())
        }),
        Err(StoreError::Conflict(_))
    ));
    assert!(!ran);
    assert_eq!(
        store
            .finish_job_fenced(
                &claimed.job_id,
                "worker",
                claimed.fencing_token,
                JobState::Failed
            )
            .unwrap()
            .state,
        JobState::Failed
    );
}

#[test]
fn withdrawn_live_permission_prevents_publication_callback_and_preserves_failed_state() {
    let (mut store, scope) = fixture();
    let queued = store
        .create_job_with_authority(
            &scope,
            JobKind::Index,
            &remote(now() + 60, vec![Permission::IndexWrite]),
            16,
        )
        .unwrap()
        .unwrap();
    let claimed = store
        .claim_job_once_strict(&queued.job_id, "worker")
        .unwrap();
    store
        .revoke_grant(&principal(), &Permission::IndexWrite, &scope)
        .unwrap();
    let mut ran = false;
    assert!(
        store
            .with_job_fence(&claimed.job_id, "worker", claimed.fencing_token, || {
                ran = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!ran);
    assert!(
        store
            .heartbeat_fenced(&claimed.job_id, "worker", claimed.fencing_token)
            .is_err()
    );
    assert_eq!(
        store
            .finish_job_fenced(
                &claimed.job_id,
                "worker",
                claimed.fencing_token,
                JobState::Failed
            )
            .unwrap()
            .state,
        JobState::Failed
    );
}

#[test]
fn strict_legacy_unknown_does_not_steal_live_running_lease() {
    let (mut store, scope) = fixture();
    let live = store
        .create_job(&scope, JobKind::Index, &principal())
        .unwrap();
    let live = store.claim_job_once(&live.job_id, "old-owner").unwrap();
    let other = PrincipalId::new("queued-subject").unwrap();
    let queued = store.create_job(&scope, JobKind::Sync, &other).unwrap();
    assert_eq!(store.job_request_authority(&live.job_id).unwrap(), None);
    assert!(matches!(
        store.claim_job_once_strict(&live.job_id, "new-owner"),
        Err(StoreError::StaleOwner)
    ));
    assert_eq!(store.job(&live.job_id).unwrap(), live);
    assert!(matches!(
        store.claim_job_once_strict(&queued.job_id, "strict"),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(store.job(&queued.job_id).unwrap().state, JobState::Failed);
    store
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&live.job_id],
        )
        .unwrap();
    assert!(matches!(
        store.claim_job_once_strict(&live.job_id, "new-owner"),
        Err(StoreError::Conflict(_))
    ));
    let expired = store.job(&live.job_id).unwrap();
    assert_eq!(expired.state, JobState::Failed);
    assert_eq!(expired.fencing_token, live.fencing_token);
    assert_eq!(expired.owner, "old-owner");
}

#[test]
fn malformed_persisted_authority_never_becomes_legacy_or_local() {
    for field in ["principal", "origin", "expires_at_unix_seconds"] {
        let (mut store, scope) = fixture();
        let job = store
            .create_job(&scope, JobKind::Index, &principal())
            .unwrap();
        let mut value =
            serde_json::to_value(remote(now() + 60, vec![Permission::IndexWrite])).unwrap();
        value[field] = match field {
            "principal" => serde_json::json!("another-subject"),
            "origin" => serde_json::json!("trusted_local"),
            _ => serde_json::Value::Null,
        };
        store
            .connection
            .execute(
                "INSERT INTO job_request_authorities VALUES(?1,1,?2)",
                rusqlite::params![job.job_id, serde_json::to_string(&value).unwrap()],
            )
            .unwrap();
        assert!(matches!(
            store.job_request_authority(&job.job_id),
            Err(StoreError::InvalidGraph(_))
        ));
        assert!(matches!(
            store.claim_job_once(&job.job_id, "trusted"),
            Err(StoreError::InvalidGraph(_))
        ));
        assert_eq!(store.job(&job.job_id).unwrap().state, JobState::Queued);
    }
}

#[test]
fn authority_row_is_immutable_but_real_parent_deletion_can_cascade() {
    let (mut store, scope) = fixture();
    let job = store
        .create_job_with_authority(
            &scope,
            JobKind::Index,
            &remote(now() + 60, vec![Permission::IndexWrite]),
            16,
        )
        .unwrap()
        .unwrap();
    assert!(
        store
            .connection
            .execute(
                "UPDATE job_request_authorities SET authority_json='{}' WHERE job_id=?1",
                [&job.job_id]
            )
            .is_err()
    );
    assert!(
        store
            .connection
            .execute(
                "DELETE FROM job_request_authorities WHERE job_id=?1",
                [&job.job_id]
            )
            .is_err()
    );
    assert_eq!(
        store
            .connection
            .execute("DELETE FROM jobs WHERE job_id=?1", [&job.job_id])
            .unwrap(),
        1
    );
    let count: i64 = store
        .connection
        .query_row("SELECT COUNT(*) FROM job_request_authorities", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}
