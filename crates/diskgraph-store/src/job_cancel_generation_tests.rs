//! 固定真实认领代次的停止资格；来源：原生 Rust ControlStore 取消协议。
//! 新接口尚未落生产时只能记录接口缺失，不能把编译失败当成行为 RED。

use crate::{ControlStore, JobKind, JobRecord, JobState, StoreError};
use diskgraph_core::{Grant, Locator, Permission, PrincipalId, ScopeId};

fn fixture() -> (
    tempfile::TempDir,
    ControlStore,
    ScopeId,
    PrincipalId,
    JobRecord,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ControlStore::open(&dir.path().join("control.sqlite")).unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(dir.path()), None)
        .unwrap();
    let actor = PrincipalId::new("generation-cancel").unwrap();
    store.publish_policy_version(1).unwrap();
    for permission in [Permission::IndexWrite, Permission::OperationView] {
        store
            .upsert_grant(&Grant {
                principal: actor.clone(),
                permission,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
    }
    let job = store.create_job(&scope, JobKind::Index, &actor).unwrap();
    (dir, store, scope, actor, job)
}

#[test]
fn current_generation_cancel_survives_operation_view_revocation_without_renewal() {
    let (dir, mut store, scope, actor, queued) = fixture();
    let running = store
        .claim_job_once(&queued.job_id, "owned-worker")
        .unwrap();
    store
        .revoke_grant(&actor, &Permission::OperationView, &scope)
        .unwrap();
    assert_eq!(
        store
            .live_permission(&actor, &Permission::OperationView, &scope)
            .unwrap(),
        Some(false)
    );
    assert!(
        store
            .request_cancel_generation(&running.job_id, &running.owner, running.fencing_token)
            .unwrap()
    );
    assert!(
        store
            .request_cancel_generation(&running.job_id, &running.owner, running.fencing_token)
            .unwrap()
    );
    assert_eq!(store.job(&running.job_id).unwrap(), running);
    assert!(
        store
            .cancellation_requested(&running.job_id, running.fencing_token)
            .unwrap()
    );
    assert!(matches!(
        store.with_job_fence(
            &running.job_id,
            &running.owner,
            running.fencing_token,
            || Ok(())
        ),
        Err(StoreError::StaleOwner)
    ));
    assert_eq!(
        store
            .live_permission(&actor, &Permission::OperationView, &scope)
            .unwrap(),
        Some(false)
    );
    drop(store);
    let reopened = ControlStore::open(&dir.path().join("control.sqlite")).unwrap();
    assert_eq!(reopened.job(&running.job_id).unwrap(), running);
    assert!(
        reopened
            .cancellation_requested(&running.job_id, running.fencing_token)
            .unwrap()
    );
}

#[test]
fn revoked_scope_still_allows_deny_only_stop_of_the_exact_running_generation() {
    let (_dir, mut store, scope, actor, queued) = fixture();
    let running = store
        .claim_job_once(&queued.job_id, "revoked-worker")
        .unwrap();
    store.revoke_scope(&scope).unwrap();
    assert!(store.scope_revoked(&scope).unwrap());
    assert!(
        store
            .request_cancel_generation(&running.job_id, &running.owner, running.fencing_token)
            .unwrap()
    );
    assert_eq!(store.job(&running.job_id).unwrap(), running);
    assert_eq!(
        store
            .live_permission(&actor, &Permission::IndexWrite, &scope)
            .unwrap(),
        Some(false)
    );
    assert!(matches!(
        store.heartbeat_fenced(&running.job_id, &running.owner, running.fencing_token),
        Err(StoreError::Conflict(message)) if message == "job cancellation requested"
    ));
    assert_eq!(store.job(&running.job_id).unwrap(), running);
}

#[test]
fn an_old_permit_cannot_cancel_a_reclaimed_generation_with_the_same_owner_name() {
    let (_dir, mut store, _scope, _actor, queued) = fixture();
    let old = store
        .claim_job_once(&queued.job_id, "reused-owner")
        .unwrap();
    // 仅将原租约设置为已过期；下一代仍经真实公开条件认领取得。
    store
        .connection
        .execute(
            "UPDATE jobs SET lease_expires_unix_ms=0 WHERE job_id=?1",
            [&old.job_id],
        )
        .unwrap();
    let current = store
        .claim_job_once(&queued.job_id, "reused-owner")
        .unwrap();
    assert_eq!(old.owner, current.owner);
    assert!(current.fencing_token > old.fencing_token);
    assert!(
        !store
            .request_cancel_generation(&old.job_id, &old.owner, old.fencing_token)
            .unwrap()
    );
    assert_eq!(store.job(&current.job_id).unwrap(), current);
    assert!(
        !store
            .cancellation_requested(&current.job_id, current.fencing_token)
            .unwrap()
    );
    store
        .with_job_fence(
            &current.job_id,
            &current.owner,
            current.fencing_token,
            || Ok(()),
        )
        .unwrap();
}

#[test]
fn wrong_owner_fence_and_unrepresentable_fence_have_no_mutation() {
    let (_dir, mut store, _scope, _actor, queued) = fixture();
    let current = store
        .claim_job_once(&queued.job_id, "current-owner")
        .unwrap();
    for (owner, fence) in [
        ("other-owner", current.fencing_token),
        (current.owner.as_str(), current.fencing_token + 1),
        (current.owner.as_str(), u64::MAX),
    ] {
        assert!(
            !store
                .request_cancel_generation(&current.job_id, owner, fence)
                .unwrap()
        );
        assert_eq!(store.job(&current.job_id).unwrap(), current);
        assert!(
            !store
                .cancellation_requested(&current.job_id, current.fencing_token)
                .unwrap()
        );
    }
}

#[test]
fn queued_terminal_and_missing_jobs_are_not_generation_cancelled() {
    let (_dir, mut store, _scope, _actor, queued) = fixture();
    assert!(
        !store
            .request_cancel_generation(&queued.job_id, "", queued.fencing_token)
            .unwrap()
    );
    assert_eq!(store.job(&queued.job_id).unwrap(), queued);
    assert!(
        !store
            .cancellation_requested(&queued.job_id, queued.fencing_token)
            .unwrap()
    );
    let running = store
        .claim_job_once(&queued.job_id, "completed-owner")
        .unwrap();
    let completed = store
        .finish_job_fenced(
            &running.job_id,
            &running.owner,
            running.fencing_token,
            JobState::Completed,
        )
        .unwrap();
    assert!(
        !store
            .request_cancel_generation(&completed.job_id, &completed.owner, completed.fencing_token)
            .unwrap()
    );
    assert_eq!(store.job(&completed.job_id).unwrap(), completed);
    assert!(
        !store
            .cancellation_requested(&completed.job_id, completed.fencing_token)
            .unwrap()
    );
    assert!(
        !store
            .request_cancel_generation("missing-job", "missing-owner", 1)
            .unwrap()
    );
}

#[test]
fn ordinary_authorized_cancel_keeps_its_queued_compatibility() {
    let (_dir, mut store, _scope, _actor, queued) = fixture();
    assert!(store.request_cancel(&queued.job_id).unwrap());
    let cancelled = store.job(&queued.job_id).unwrap();
    assert_eq!(cancelled.state, JobState::Cancelled);
    assert_eq!(cancelled.fencing_token, queued.fencing_token);
    assert!(
        !store
            .request_cancel_generation(&cancelled.job_id, "", cancelled.fencing_token)
            .unwrap()
    );
    assert_eq!(store.job(&cancelled.job_id).unwrap(), cancelled);
}
