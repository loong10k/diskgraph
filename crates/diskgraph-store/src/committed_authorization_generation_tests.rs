//! 本地提交下界的真实SQLite提交、回滚与原连接代次测试；不证明跨连接原生通知。
use crate::ControlStore;
use diskgraph_core::{Grant, Locator, Permission, PrincipalId, ScopeId};

fn fixture() -> (ControlStore, PrincipalId, ScopeId) {
    let mut store = ControlStore::open_in_memory().unwrap();
    let scope = store
        .register_scope(&Locator::from_native_path(&std::env::temp_dir()), None)
        .unwrap();
    let actor = PrincipalId::new("committed-generation-reader").unwrap();
    store.publish_policy_version(1).unwrap();
    store
        .upsert_grant(&Grant {
            principal: actor.clone(),
            scope: scope.clone(),
            permission: Permission::MetadataRead,
            policy_version: 1,
        })
        .unwrap();
    (store, actor, scope)
}

#[test]
fn local_commit_is_known_without_a_native_database_file_identity() {
    let (mut store, actor, scope) = fixture();
    assert!(store.withdrawal_incarnation.identity().is_none());
    let before = store.authorization_generation().unwrap();
    let watch = store.committed_authorization_generation_witness();
    assert_eq!(watch.latest_known_for(&store), Some(0));
    store
        .revoke_grant(&actor, &Permission::MetadataRead, &scope)
        .unwrap();
    let known = watch.latest_known_for(&store).unwrap();
    assert!(known > before);
    assert_eq!(known, store.authorization_generation().unwrap());
}

#[test]
fn deferred_foreign_key_commit_failure_does_not_publish_a_memory_fact() {
    let (mut store, actor, scope) = fixture();
    let before = store.authorization_generation().unwrap();
    let watch = store.committed_authorization_generation_witness();
    // DELETE与授权代次已在事务中发生；真正COMMIT因延迟外键失败并回滚。
    store
        .connection
        .execute_batch(
            "CREATE TABLE failed_commit_guard (
          value TEXT REFERENCES scopes(scope_id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER fail_grant_commit AFTER DELETE ON grants BEGIN
          INSERT INTO failed_commit_guard VALUES ('missing-scope'); END;",
        )
        .unwrap();
    assert!(
        store
            .revoke_grant(&actor, &Permission::MetadataRead, &scope)
            .is_err()
    );
    assert_eq!(watch.latest_known_for(&store), Some(0));
    assert_eq!(store.authorization_generation().unwrap(), before);
    assert_eq!(
        store
            .live_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(true)
    );
}

#[test]
fn old_connection_weak_witness_cannot_match_a_new_connection() {
    let (mut original, actor, scope) = fixture();
    let watch = original.committed_authorization_generation_witness();
    original
        .revoke_grant(&actor, &Permission::MetadataRead, &scope)
        .unwrap();
    assert!(
        watch
            .latest_known_for(&original)
            .is_some_and(|generation| generation > 0)
    );
    drop(original);
    let (mut reopened, actor, scope) = fixture();
    reopened
        .revoke_grant(&actor, &Permission::MetadataRead, &scope)
        .unwrap();
    assert_eq!(watch.latest_known_for(&reopened), None);
}
