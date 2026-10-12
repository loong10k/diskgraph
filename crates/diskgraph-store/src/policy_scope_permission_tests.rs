//! 原授权组合的兼容、实时撤权与单次 SQL 观察回归；全部使用隔离控制库。
use crate::{ControlStore, StoreError};
use diskgraph_core::{Locator, Permission, PrincipalId, ScopeId};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn fixture(store: &mut ControlStore) -> (PrincipalId, ScopeId) {
    let scope = store
        .register_scope(&Locator::from_document_uri("content://atomic-policy"), None)
        .unwrap();
    (PrincipalId::new("actor").unwrap(), scope)
}

#[test]
fn scope_capability_observes_policy_scope_and_grant_in_one_sql_statement() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = fixture(&mut store);
    store.publish_policy_version(1).unwrap();
    store.connection.set_prepared_statement_cache_capacity(0);
    let selects = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&selects);
    store
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(context.action, AuthAction::Select) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Authorization::Allow
        }))
        .unwrap();
    assert_eq!(
        store
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    // 一个顶层 SELECT 加一个 EXISTS 子查询；单独策略探针会产生第三次真实编译。
    assert_eq!(
        selects.load(Ordering::Relaxed),
        2,
        "scope capability still probes policy separately"
    );
}

#[test]
fn scope_capability_preserves_unpublished_and_exact_grant_matrix() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = fixture(&mut store);
    let missing = ScopeId::new("missing").unwrap();
    assert_eq!(
        store
            .policy_scope_permission(&actor, &Permission::MetadataRead, &missing)
            .unwrap(),
        None
    );
    for version in [-1_i64, 0, 1, 2] {
        for revoked in [0, 1] {
            for scope_revoked in [0, 1] {
                store
                    .connection
                    .execute(
                        "INSERT OR REPLACE INTO policy VALUES(1,?1,?2)",
                        rusqlite::params![version, revoked],
                    )
                    .unwrap();
                store
                    .connection
                    .execute(
                        "UPDATE scopes SET revoked=?1 WHERE scope_id=?2",
                        rusqlite::params![scope_revoked, scope.as_str()],
                    )
                    .unwrap();
                store
                    .connection
                    .execute(
                        "INSERT OR REPLACE INTO grants VALUES('actor','metadata:read',?1,?2)",
                        rusqlite::params![scope.as_str(), version],
                    )
                    .unwrap();
                for actor in ["actor", "other"] {
                    for permission in [Permission::MetadataRead, Permission::ContentRead] {
                        let principal = PrincipalId::new(actor).unwrap();
                        let expected = store
                            .live_permission(&principal, &permission, &scope)
                            .unwrap();
                        assert_eq!(
                            store
                                .policy_scope_permission(&principal, &permission, &scope)
                                .unwrap(),
                            expected
                        );
                    }
                }
            }
        }
    }
    assert!(matches!(
        store.policy_scope_permission(&actor, &Permission::MetadataRead, &missing),
        Err(StoreError::ScopeNotFound(_))
    ));
    store
        .connection
        .execute_batch("DELETE FROM policy; UPDATE scopes SET revoked=1")
        .unwrap();
    assert_eq!(
        store
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        None
    );
}

#[test]
fn scope_capability_rejects_corrupt_policy_before_missing_or_revoked_scope() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = fixture(&mut store);
    let missing = ScopeId::new("missing").unwrap();
    for column in ["version", "revoked"] {
        store
            .connection
            .execute_batch(
                "INSERT OR REPLACE INTO policy VALUES(1,1,0); UPDATE scopes SET revoked=1",
            )
            .unwrap();
        store
            .connection
            .execute(&format!("UPDATE policy SET {column}='corrupt'"), [])
            .unwrap();
        for scope in [&scope, &missing] {
            assert!(matches!(
                store.policy_scope_permission(&actor, &Permission::MetadataRead, scope),
                Err(StoreError::Sqlite(_))
            ));
        }
    }
}

#[test]
fn scope_capability_warm_statement_observes_independent_grant_and_scope_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut reader = ControlStore::open(&path).unwrap();
    let (actor, scope) = fixture(&mut reader);
    reader.publish_policy_version(1).unwrap();
    let writer = ControlStore::open(&path).unwrap();
    writer
        .connection
        .execute(
            "INSERT INTO grants VALUES('actor','metadata:read',?1,1)",
            [scope.as_str()],
        )
        .unwrap();
    assert_eq!(
        reader
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(true)
    );
    writer
        .connection
        .execute_batch("DELETE FROM grants")
        .unwrap();
    assert_eq!(
        reader
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    writer
        .connection
        .execute(
            "INSERT INTO grants VALUES('actor','metadata:read',?1,1)",
            [scope.as_str()],
        )
        .unwrap();
    assert_eq!(
        reader
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(true)
    );
    writer
        .connection
        .execute(
            "UPDATE scopes SET revoked=1 WHERE scope_id=?1",
            [scope.as_str()],
        )
        .unwrap();
    assert_eq!(
        reader
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
}

#[test]
fn scope_capability_still_executes_vm_and_honors_original_deadline() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = fixture(&mut store);
    store.publish_policy_version(1).unwrap();
    assert_eq!(
        store
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    store.connection.progress_handler(1, Some(|| true)).unwrap();
    assert!(
        store
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap_err()
            .is_interrupted()
    );
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    let result: Result<Option<bool>, StoreError> = store
        .with_read_deadline(Instant::now() - Duration::from_millis(1), |store| {
            store.policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
        });
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
    assert_eq!(
        store
            .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
}

#[test]
#[ignore = "显式 release 微基准；仅测量隔离内存库权限读取，不代表端到端性能"]
fn scope_capability_release_comparison() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = fixture(&mut store);
    store.publish_policy_version(1).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO grants VALUES('actor','metadata:read',?1,1)",
            [scope.as_str()],
        )
        .unwrap();
    for (round, atomic) in [false, true, true, false].into_iter().enumerate() {
        let mut samples = Vec::with_capacity(2000);
        for index in 0..2100 {
            let started = Instant::now();
            let allowed = if atomic {
                store
                    .policy_scope_permission(&actor, &Permission::MetadataRead, &scope)
                    .unwrap()
            } else if store.policy_state().unwrap().is_some() {
                store
                    .live_permission(&actor, &Permission::MetadataRead, &scope)
                    .unwrap()
            } else {
                None
            };
            let elapsed = started.elapsed().as_nanos();
            assert_eq!(allowed, Some(true));
            if index >= 100 {
                samples.push(elapsed);
            }
        }
        samples.sort_unstable();
        eprintln!(
            "round={round} atomic={atomic} samples={} p50_ns={} p95_ns={}",
            samples.len(),
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100]
        );
    }
}
