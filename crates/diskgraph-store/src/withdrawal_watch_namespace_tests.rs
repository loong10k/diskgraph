//! 实际数据库与连接代次隔离，不使用相同路径或 server 作为 namespace。
use super::withdrawal_test_fixture::WithdrawalFixture;
use crate::{AuthorizationWithdrawalStatus, ControlStore, StoreError};
use diskgraph_core::Permission;

#[test]
fn backup_with_the_same_server_cannot_withdraw_the_original_database_request() {
    let mut f = WithdrawalFixture::new();
    let mut backup = f.backup();
    assert_eq!(
        f.store.ensure_server().unwrap(),
        backup.ensure_server().unwrap()
    );
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    backup.revoke_scope(&f.scope).unwrap();
    assert!(backup.scope_revoked(&f.scope).unwrap());
    assert!(!f.store.scope_revoked(&f.scope).unwrap());
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
fn moved_store_keeps_incarnation_but_reopening_the_same_file_invalidates_old_watch() {
    let f = WithdrawalFixture::new();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    let WithdrawalFixture {
        store,
        path,
        directory,
        ..
    } = f;
    let moved_store = Box::new(store);
    assert_eq!(
        watch.status_for(&moved_store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    let reopened = ControlStore::open(&path).unwrap();
    assert_eq!(
        watch.status_for(&reopened),
        AuthorizationWithdrawalStatus::Invalidated,
        "even the same physical file is not the request's original connection incarnation"
    );
    drop(moved_store);
    drop(reopened);
    std::fs::rename(&path, directory.path().join("old.sqlite")).unwrap();
    let replacement = ControlStore::open(&path).unwrap();
    assert_eq!(
        watch.status_for(&replacement),
        AuthorizationWithdrawalStatus::Invalidated
    );
}

#[test]
fn replacing_a_live_store_never_consumes_the_previous_withdrawn_flag() {
    let mut f = WithdrawalFixture::new();
    let watch = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    f.second()
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    f.store = ControlStore::open(&f.path).unwrap();
    assert_eq!(
        watch.status_for(&f.store),
        AuthorizationWithdrawalStatus::Invalidated
    );
}

#[test]
fn per_database_capacity_is_bounded_and_dead_weak_entries_are_reclaimed() {
    let f = WithdrawalFixture::new();
    let mut watches = Vec::new();
    for _ in 0..256 {
        watches.push(
            f.store
                .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
                .unwrap()
                .unwrap(),
        );
    }
    assert!(matches!(
        f.store
            .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead),
        Err(StoreError::BudgetExceeded)
    ));
    drop(watches.pop());
    let replacement = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .unwrap();
    f.second()
        .revoke_grant(&f.principal, &Permission::MetadataRead, &f.scope)
        .unwrap();
    assert_eq!(
        replacement.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    assert!(
        watches
            .iter()
            .all(|watch| watch.status_for(&f.store) == AuthorizationWithdrawalStatus::Withdrawn)
    );
    drop(watches);
    drop(replacement);
    for _ in 0..1024 {
        let watch = f
            .store
            .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
            .unwrap()
            .unwrap();
        assert_eq!(
            watch.status_for(&f.store),
            AuthorizationWithdrawalStatus::Unchanged
        );
    }
}
