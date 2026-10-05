//! 独立 registry 验证进程总额；不依赖其它并行测试恰好释放生产全局配额。
use super::withdrawal_registry::WithdrawalRegistry;
use super::withdrawal_test_fixture::WithdrawalFixture;
use crate::{AuthorizationWithdrawalStatus, StoreError};
use diskgraph_core::Permission;

#[test]
fn isolated_registry_enforces_4096_live_watches_across_real_databases_and_reclaims_weak_slots() {
    let fixtures: Vec<_> = (0..17).map(|_| WithdrawalFixture::new()).collect();
    let mut registry = WithdrawalRegistry::new();
    let mut watches = Vec::new();
    for f in &fixtures[..16] {
        let epoch = f.store.policy_state().unwrap().map(|(epoch, _)| epoch);
        let generation = f.store.authorization_generation().unwrap();
        assert_eq!(epoch, Some(1));
        for _ in 0..256 {
            watches.push(
                registry
                    .register(
                        &f.store,
                        &f.principal,
                        &f.scope,
                        &Permission::MetadataRead,
                        epoch,
                        generation,
                    )
                    .unwrap()
                    .expect("qualified real Windows main DB"),
            );
        }
    }
    let last = &fixtures[16];
    let epoch = last.store.policy_state().unwrap().map(|(epoch, _)| epoch);
    let generation = last.store.authorization_generation().unwrap();
    assert!(matches!(
        registry.register(
            &last.store,
            &last.principal,
            &last.scope,
            &Permission::MetadataRead,
            epoch,
            generation
        ),
        Err(StoreError::BudgetExceeded)
    ));
    // 原满额全部是强 watch；只释放一个请求，就必须能准入其它真实数据库。
    drop(watches.pop());
    let replacement = registry
        .register(
            &last.store,
            &last.principal,
            &last.scope,
            &Permission::MetadataRead,
            epoch,
            generation,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        replacement.status_for(&last.store),
        AuthorizationWithdrawalStatus::Unchanged
    );
    assert!(matches!(
        registry.register(
            &last.store,
            &last.principal,
            &last.scope,
            &Permission::MetadataRead,
            epoch,
            generation
        ),
        Err(StoreError::BudgetExceeded)
    ));
    drop(watches);
    drop(replacement);
    // 死 Weak 不得成为永久容量占用；同一个 registry 再完成一轮满额准入。
    let mut second_generation = Vec::new();
    for f in &fixtures[..16] {
        let epoch = f.store.policy_state().unwrap().map(|(epoch, _)| epoch);
        let generation = f.store.authorization_generation().unwrap();
        for _ in 0..256 {
            second_generation.push(
                registry
                    .register(
                        &f.store,
                        &f.principal,
                        &f.scope,
                        &Permission::MetadataRead,
                        epoch,
                        generation,
                    )
                    .unwrap()
                    .unwrap(),
            );
        }
    }
    assert_eq!(second_generation.len(), 4096);
}
