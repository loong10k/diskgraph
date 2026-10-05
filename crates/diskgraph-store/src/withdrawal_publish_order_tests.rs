//! 真实 FULL 提交后通知延迟：新请求不得被已经发生的撤权污染，旧请求仍单调停止。
use crate::AuthorizationWithdrawalStatus;
use crate::withdrawal_publish_hook::WithdrawalPublishHook;
use crate::withdrawal_test_fixture::WithdrawalFixture;
use diskgraph_core::Permission;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn delayed_committed_grant_notice_does_not_withdraw_a_regranted_new_request() {
    let mut f = WithdrawalFixture::new();
    let old = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .expect("real Windows main identity required");
    let before = f.store.authorization_generation().unwrap();
    let mut writer = f.second();
    let principal = f.principal.clone();
    let scope = f.scope.clone();
    let (committed_tx, committed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = std::thread::spawn(move || {
        let _hook = WithdrawalPublishHook::install(move || {
            committed_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        writer.revoke_grant(&principal, &Permission::MetadataRead, &scope)
    });
    let reached = committed_rx.recv_timeout(Duration::from_secs(5));
    // 所有资格检查先保留结果，主动释放并 join 后才断言，失败也不遗留屏障线程。
    let observations = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reached.as_ref().ok().map(|_| {
            let committed = f.store.authorization_generation();
            let denied = f
                .store
                .live_permission(&f.principal, &Permission::MetadataRead, &f.scope);
            let pending = old.status_for(&f.store);
            f.regrant();
            let new_generation = f.store.authorization_generation();
            let fresh = f.store.watch_authorization_withdrawal(
                &f.principal,
                &f.scope,
                &Permission::MetadataRead,
            );
            (committed, denied, pending, new_generation, fresh)
        })
    }));
    let released = release_tx.send(());
    let outcome = task.join();
    assert!(
        reached.is_ok(),
        "actual post-commit hook not reached: {reached:?}"
    );
    assert!(released.is_ok());
    outcome.unwrap().unwrap();
    let (committed, denied, pending, new_generation, fresh) = observations.unwrap().unwrap();
    let committed = committed.unwrap();
    assert!(committed > before);
    assert_eq!(denied.unwrap(), Some(false));
    assert_eq!(pending, AuthorizationWithdrawalStatus::Unchanged);
    assert!(new_generation.unwrap() > committed);
    assert_eq!(
        old.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    assert_eq!(
        fresh.unwrap().unwrap().status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(true)
    );
}

#[test]
fn delayed_committed_scope_notice_does_not_withdraw_a_new_reenabled_request() {
    let f = WithdrawalFixture::new();
    let old = f
        .store
        .watch_authorization_withdrawal(&f.principal, &f.scope, &Permission::MetadataRead)
        .unwrap()
        .expect("real Windows main identity required");
    let before = f.store.authorization_generation().unwrap();
    let mut writer = f.second();
    let scope = f.scope.clone();
    let (committed_tx, committed_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let task = std::thread::spawn(move || {
        let _hook = WithdrawalPublishHook::install(move || {
            committed_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        writer.revoke_scope(&scope)
    });
    let reached = committed_rx.recv_timeout(Duration::from_secs(5));
    let observations = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reached.as_ref().ok().map(|_| {
            let committed = f.store.authorization_generation();
            let revoked = f.store.scope_revoked(&f.scope);
            let pending = old.status_for(&f.store);
            // 测试用真实 SQL 模拟管理端重新启用；generation 仍由正式触发器生成，绝不手填。
            let changed = f.store.connection.execute(
                "UPDATE scopes SET revoked=0 WHERE scope_id=?1",
                [f.scope.as_str()],
            );
            let new_generation = f.store.authorization_generation();
            let fresh = f.store.watch_authorization_withdrawal(
                &f.principal,
                &f.scope,
                &Permission::MetadataRead,
            );
            (committed, revoked, pending, changed, new_generation, fresh)
        })
    }));
    let released = release_tx.send(());
    let outcome = task.join();
    assert!(
        reached.is_ok(),
        "actual post-commit hook not reached: {reached:?}"
    );
    assert!(released.is_ok());
    outcome.unwrap().unwrap();
    let (committed, revoked, pending, changed, new_generation, fresh) =
        observations.unwrap().unwrap();
    let committed = committed.unwrap();
    assert!(committed > before);
    assert!(revoked.unwrap());
    assert_eq!(pending, AuthorizationWithdrawalStatus::Unchanged);
    assert_eq!(changed.unwrap(), 1);
    assert!(new_generation.unwrap() > committed);
    assert_eq!(
        old.status_for(&f.store),
        AuthorizationWithdrawalStatus::Withdrawn
    );
    assert_eq!(
        fresh.unwrap().unwrap().status_for(&f.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    assert_eq!(
        f.store
            .live_permission(&f.principal, &Permission::MetadataRead, &f.scope)
            .unwrap(),
        Some(true)
    );
}
