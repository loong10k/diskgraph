//! D45 成功持久提交后才发布负向事实；新接口缺失只记 interface absence。
use super::withdrawal_test_fixture::WithdrawalFixture;
use crate::{AuthorizationWithdrawalStatus, StoreError};
use diskgraph_core::{Grant, Locator, Permission, PrincipalId};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[test]
fn committed_current_grant_withdrawal_is_monotone_for_only_the_existing_request() {
    let mut f = WithdrawalFixture::new();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .expect("qualified Windows watch");
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    f.second()
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    f.regrant();
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(true)
    );
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    let fresh = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    assert_eq!(
        fresh.status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
}

#[test]
fn unrelated_principal_scope_permission_and_delete_zero_never_mark_withdrawn() {
    let mut f = WithdrawalFixture::new();
    let other_root = f.directory.path().join("other-root");
    std::fs::create_dir(&other_root).unwrap();
    let other_scope = f
        .store
        .register_scope(&Locator::from_native_path(&other_root), None)
        .unwrap();
    let other_principal = PrincipalId::new("other-reader").unwrap();
    for (principal, scope, permission) in [
        (other_principal, f.scope.clone(), Permission::MetadataRead),
        (
            f.principal.clone(),
            other_scope.clone(),
            Permission::MetadataRead,
        ),
        (f.principal.clone(), f.scope.clone(), Permission::IndexWrite),
    ] {
        f.store
            .upsert_grant(&Grant {
                principal: principal.clone(),
                permission,
                scope: scope.clone(),
                policy_version: 1,
            })
            .unwrap();
        let watch = f
            .store
            .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
            .unwrap()
            .unwrap();
        f.second()
            .revoke_grant(&principal, &permission, &scope)
            .unwrap();
        assert_eq!(
            watch.status_for(&f.store),
            AuthorizationWithdrawalStatus::Unchanged
        );
    }
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    f.second().revoke_scope(&other_scope).unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    // 真实 raw SQL 移除原grant，不经过通知入口；随后公开 DELETE0 不得补造一个提交撤权事件。
    f.store
        .connection
        .execute("DELETE FROM grants", [])
        .unwrap();
    f.second()
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
}

#[test]
fn deleting_only_non_current_epoch_rows_does_not_manufacture_denial() {
    let mut f = WithdrawalFixture::new();
    f.store
        .connection
        .execute("DELETE FROM grants", [])
        .unwrap();
    f.store
        .upsert_grant(&Grant {
            principal: f.principal.clone(),
            permission: Permission::MetadataRead,
            scope: f.scope.clone(),
            policy_version: 2,
        })
        .unwrap();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
    let mut second = f.second();
    second
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    assert!(
        second.all_grants().unwrap().is_empty(),
        "a real historical/future row was deleted"
    );
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
}

#[test]
fn legacy_scope_only_watch_ignores_unused_grant_but_observes_committed_scope_revoke() {
    let f = WithdrawalFixture::new();
    f.store
        .connection
        .execute("DELETE FROM policy", [])
        .unwrap();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        None
    );
    f.second()
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    f.second().revoke_scope(&f.scope).unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
}

#[test]
fn real_commit_abort_does_not_publish_grant_or_scope_withdrawal() {
    for scope_revoke in [false, true] {
        let f = WithdrawalFixture::new();
        let watch = f
            .store
            .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
            .unwrap()
            .unwrap();
        let mut second = f.second();
        let reached = Arc::new(AtomicBool::new(false));
        let mark = Arc::clone(&reached);
        second
            .connection
            .commit_hook(Some(move || {
                mark.store(true, Ordering::SeqCst);
                true
            }))
            .unwrap();
        let result = if scope_revoke {
            second.revoke_scope(&f.scope)
        } else {
            second.revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        };
        second.connection.commit_hook(None::<fn() -> bool>).unwrap();
        assert!(
            reached.load(Ordering::SeqCst),
            "real COMMIT hook must be reached"
        );
        assert!(
            matches!(result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == rusqlite::ErrorCode::ConstraintViolation)
        );
        assert!(!f.store.scope_revoked(&f.scope).unwrap());
        assert_eq!(
            f.store
                .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
                .unwrap(),
            Some(true)
        );
        assert_eq!(
            watch.status_for(&f.store),
            AuthorizationWithdrawalStatus::Unchanged
        );
    }
}

#[test]
fn successful_scope_commit_is_observed_even_when_the_following_reap_fails() {
    let f = WithdrawalFixture::new();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    let mut second = f.second();
    let reached = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&reached);
    second
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(
                context.action,
                AuthAction::Update {
                    table_name: "jobs",
                    column_name: "heartbeat_unix_ms"
                }
            ) {
                mark.store(true, Ordering::SeqCst);
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
    let result = second.revoke_scope(&f.scope);
    second
        .connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert!(
        reached.load(Ordering::SeqCst),
        "real post-commit reap SQL must be reached"
    );
    assert!(
        matches!(result, Err(StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _))) if error.code == rusqlite::ErrorCode::AuthorizationForStatementDenied)
    );
    assert!(
        f.store.scope_revoked(&f.scope).unwrap(),
        "scope COMMIT actually persisted despite later Err"
    );
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
}

#[test]
fn pure_status_preserves_known_withdrawal_without_sql_after_the_original_deadline() {
    let f = WithdrawalFixture::new();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    f.second()
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    while std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let sql_seen = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&sql_seen);
    f.store
        .connection
        .authorizer(Some(move |_: AuthContext<'_>| {
            mark.store(true, Ordering::SeqCst);
            Authorization::Deny
        }))
        .unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    assert!(
        !sql_seen.load(Ordering::SeqCst),
        "pure witness must not bypass the expired SQL deadline by doing SQL"
    );
    f.store
        .connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert!(
        matches!(
            f.store
                .with_read_deadline(deadline, |store| store.scope_revoked(&f.scope)),
            Err(StoreError::BudgetExceeded)
        ),
        "the same expired SQL deadline remains a deadline; no blanket Budget-to-Denied conversion"
    );
}

#[test]
fn in_progress_real_commit_never_publishes_before_successful_return() {
    let f = WithdrawalFixture::new();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    let mut writer = f.second();
    let principal = f.principal.clone();
    let scope = f.scope.clone();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        writer
            .connection
            .commit_hook(Some(move || {
                entered_tx.send(()).unwrap();
                // 不把 hook 当提交完成；超时只中止真实事务以救援清理，不能作为通过。
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .is_err()
            }))
            .unwrap();
        writer.revoke_grant(&principal, &Permission::MetadataRead, &scope)
    });
    let entered = entered_rx.recv_timeout(std::time::Duration::from_secs(5));
    let before_commit = watch.status_for(&f.store);
    let released = release_tx.send(());
    let result = worker.join().unwrap();
    // 先释放并 join 再断言，失败也不会留下被测事务卡住。
    assert!(
        entered.is_ok(),
        "real COMMIT phase was not reached: {entered:?}"
    );
    assert!(
        released.is_ok(),
        "commit watchdog must not supply test progress"
    );
    assert_eq!(before_commit, AuthorizationWithdrawalStatus::Unchanged);
    result.unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(false)
    );
}
