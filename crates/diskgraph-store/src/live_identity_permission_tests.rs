//! 用真实 SQLite 工作量和独立提交验证长连接窄授权。
use crate::ControlStore;
use diskgraph_core::{Grant, Locator, Permission, PrincipalId, ScopeId};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn actor() -> PrincipalId {
    PrincipalId::new("stream-actor").unwrap()
}
fn admin() -> ScopeId {
    ScopeId::new("diskgraph-admin").unwrap()
}
fn grant(store: &mut ControlStore, scope: &ScopeId, version: u64) {
    store
        .upsert_grant(&Grant {
            principal: actor(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version: version,
        })
        .unwrap();
}

#[test]
fn live_identity_work_does_not_scan_other_principals_or_scopes() {
    let mut store = ControlStore::open_in_memory().unwrap();
    store.publish_policy_version(1).unwrap();
    let scope = store
        .register_scope(&Locator::from_document_uri("content://stream-owned"), None)
        .unwrap();
    grant(&mut store, &scope, 1);
    store.connection.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2000) INSERT INTO grants SELECT 'other-'||i,'metadata:read','unused-'||i,1 FROM n;").unwrap();
    store.connection.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<2000) INSERT INTO scopes(scope_id,root_kind,root_raw_b64,root_display,created_at_unix_ms) SELECT 'unused-'||i,'document_uri','root-'||i,'unrelated',0 FROM n;").unwrap();
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&steps);
    store
        .connection
        .progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )
        .unwrap();
    assert!(
        store
            .identity_has_live_permission(&actor(), &[Permission::MetadataRead], &admin())
            .unwrap()
    );
    let measured = steps.load(Ordering::Relaxed);
    println!("live_identity_sqlite_vm_steps={measured}");
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    assert!(
        measured < 1000,
        "unrelated grants inflated actual SQLite VM work: {measured}"
    );
}

#[test]
fn oversized_scope_identity_is_refused_before_owned_allocation() {
    use crate::git_raw_allocation_tests::{isolated, measure};
    let name = "policy_store::live_identity_permission::tests::oversized_scope_identity_is_refused_before_owned_allocation";
    if isolated(name) {
        return;
    }
    let mut store = ControlStore::open_in_memory().unwrap();
    store.publish_policy_version(1).unwrap();
    let oversized = "0".repeat(2 << 20);
    store.connection.execute(
        "INSERT INTO scopes(scope_id,root_kind,root_raw_b64,root_display,created_at_unix_ms) VALUES(?1,'document_uri','unused','unused',0)",
        [&oversized],
    ).unwrap();
    store.connection.execute(
        "INSERT INTO grants(principal_id,permission,scope_id,policy_version) VALUES(?1,'metadata:read',?2,1)",
        rusqlite::params![actor().as_str(), oversized],
    ).unwrap();
    let principal = actor();
    let management = admin();
    let check = |store: &ControlStore| {
        measure(|| {
            store.identity_has_live_permission(&principal, &[Permission::MetadataRead], &management)
        })
    };
    let (result, requested) = check(&store);
    assert!(!result.unwrap(), "invalid identity must never authorize");
    eprintln!("oversized 2MiB identity; Rust requested={requested}");
    assert!(
        requested < 65536,
        "invalid identity was owned before validation: {requested}"
    );
    grant(&mut store, &management, 1);
    let (result, requested) = check(&store);
    assert!(
        result.unwrap(),
        "invalid row must not hide a valid live grant"
    );
    assert!(
        requested < 65536,
        "invalid prefix inflated authorized read: {requested}"
    );
}

#[test]
fn live_identity_preserves_epoch_token_scope_and_admin_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut reader = ControlStore::open(&path).unwrap();
    let scope = reader
        .register_scope(&Locator::from_document_uri("content://stream-scope"), None)
        .unwrap();
    let check = |store: &ControlStore| {
        store
            .identity_has_live_permission(&actor(), &[Permission::MetadataRead], &admin())
            .unwrap()
    };
    grant(&mut reader, &scope, 0);
    assert!(!check(&reader));
    reader.publish_policy_version(1).unwrap();
    assert!(!check(&reader));
    grant(&mut reader, &scope, 1);
    assert!(check(&reader));
    assert!(
        !reader
            .identity_has_live_permission(&actor(), &[], &admin())
            .unwrap()
    );
    assert!(
        !reader
            .identity_has_live_permission(&actor(), &[Permission::ContentRead], &admin())
            .unwrap()
    );
    let mut writer = ControlStore::open(&path).unwrap();
    writer
        .revoke_grant(&actor(), &Permission::MetadataRead, &scope)
        .unwrap();
    assert!(!check(&reader));
    grant(&mut writer, &scope, 1);
    assert!(check(&reader));
    writer.revoke_scope(&scope).unwrap();
    assert!(!check(&reader));
    grant(&mut writer, &admin(), 1);
    assert!(
        check(&reader),
        "management grant need not have a registered scope"
    );
    writer.revoke_policy().unwrap();
    assert!(!check(&reader));
    writer.publish_policy_version(2).unwrap();
    assert!(!check(&reader));
    grant(&mut writer, &admin(), 2);
    assert!(check(&reader));
    writer
        .connection
        .progress_handler(1, Some(|| true))
        .unwrap();
    assert!(
        writer
            .identity_has_live_permission(&actor(), &[Permission::MetadataRead], &admin())
            .is_err()
    );
}

#[test]
fn scope_identity_conversion_errors_remain_errors() {
    for expression in ["x'ff'", "CAST(x'ff' AS TEXT)"] {
        let mut store = ControlStore::open_in_memory().unwrap();
        store.publish_policy_version(1).unwrap();
        // 仅固定测试表达式进入SQL；模拟真实BLOB/非法UTF-8列，不将转换失败变成有效授权。
        store
            .connection
            .execute_batch(&format!(
            "INSERT INTO scopes(scope_id,root_kind,root_raw_b64,root_display,created_at_unix_ms)
             VALUES({expression},'document_uri','unused','unused',0);
             INSERT INTO grants(principal_id,permission,scope_id,policy_version)
             VALUES('stream-actor','metadata:read',{expression},1);"
        ))
            .unwrap();
        let result =
            store.identity_has_live_permission(&actor(), &[Permission::MetadataRead], &admin());
        assert!(
            matches!(result, Err(crate::StoreError::Sqlite(_))),
            "{expression}: {result:?}"
        );
    }
}
