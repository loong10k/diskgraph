//! 控制查询复用编译产物，同时每次执行仍观察实时数据库与原预算。
use crate::{ControlStore, StoreError};
use diskgraph_core::{Grant, Locator, Permission, PrincipalId, ScopeId};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn grant_fixture(store: &mut ControlStore) -> (PrincipalId, ScopeId) {
    let actor = PrincipalId::new("cached-query-actor").unwrap();
    let scope = store
        .register_scope(&Locator::from_document_uri("content://cached-query"), None)
        .unwrap();
    store.publish_policy_version(1).unwrap();
    store
        .upsert_grant(&Grant {
            principal: actor.clone(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version: 1,
        })
        .unwrap();
    store.ensure_server().unwrap();
    (actor, scope)
}

fn read_round(store: &ControlStore, actor: &PrincipalId, scope: &ScopeId) {
    assert_eq!(
        store
            .live_permission(actor, &Permission::MetadataRead, scope)
            .unwrap(),
        Some(true)
    );
    assert!(!store.scope_revoked(scope).unwrap());
    assert!(store.authorization_generation().unwrap() > 0);
    assert!(store.existing_server_id().is_ok());
    assert_eq!(store.policy_state().unwrap(), Some((1, false)));
    assert_eq!(store.policy_version().unwrap(), 1);
}

#[test]
fn repeated_control_reads_do_not_recompile_unchanged_sql() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = grant_fixture(&mut store);
    let prepares = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&prepares);
    store
        .connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if matches!(context.action, AuthAction::Select) {
                observed.fetch_add(1, Ordering::Relaxed);
            }
            Authorization::Allow
        }))
        .unwrap();
    read_round(&store, &actor, &scope);
    let warm = prepares.load(Ordering::Relaxed);
    assert!(warm > 0, "must observe real SQLite statement compilation");
    for _ in 0..100 {
        read_round(&store, &actor, &scope);
    }
    assert_eq!(
        prepares.load(Ordering::Relaxed),
        warm,
        "unchanged SQL must reuse compilation without caching authorization results"
    );
}

#[test]
fn warm_control_queries_observe_independent_committed_revocation_and_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let mut reader = ControlStore::open(&path).unwrap();
    let (actor, scope) = grant_fixture(&mut reader);
    let mut writer = ControlStore::open(&path).unwrap();
    read_round(&reader, &actor, &scope);
    let generation = reader.authorization_generation().unwrap();
    writer
        .revoke_grant(&actor, &Permission::MetadataRead, &scope)
        .unwrap();
    assert_eq!(
        reader
            .live_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    assert!(reader.authorization_generation().unwrap() > generation);
    writer
        .upsert_grant(&Grant {
            principal: actor.clone(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version: 1,
        })
        .unwrap();
    read_round(&reader, &actor, &scope);
    let other = PrincipalId::new("not-the-warmed-actor").unwrap();
    assert_eq!(
        reader
            .live_permission(&other, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        reader
            .live_permission(&actor, &Permission::ContentRead, &scope)
            .unwrap(),
        Some(false)
    );
    writer.publish_policy_version(2).unwrap();
    assert_eq!(reader.policy_state().unwrap(), Some((2, false)));
    assert_eq!(reader.policy_version().unwrap(), 2);
    assert_eq!(
        reader
            .live_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
    writer.revoke_scope(&scope).unwrap();
    assert!(reader.scope_revoked(&scope).unwrap());
    assert_eq!(
        reader
            .live_permission(&actor, &Permission::MetadataRead, &scope)
            .unwrap(),
        Some(false)
    );
}

#[test]
fn warm_control_queries_still_execute_sqlite_progress_and_restore_after_interrupt() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = grant_fixture(&mut store);
    read_round(&store, &actor, &scope);
    store.connection.progress_handler(1, Some(|| true)).unwrap();
    let stopped = store
        .live_permission(&actor, &Permission::MetadataRead, &scope)
        .unwrap_err();
    assert!(
        stopped.is_interrupted(),
        "cached query bypassed the actual VM: {stopped}"
    );
    store
        .connection
        .progress_handler(0, None::<fn() -> bool>)
        .unwrap();
    read_round(&store, &actor, &scope);
    let result: Result<(), StoreError> =
        store.with_read_deadline(Instant::now() - Duration::from_millis(1), |store| {
            read_round(store, &actor, &scope);
            Ok(())
        });
    assert!(matches!(result, Err(StoreError::BudgetExceeded)));
    read_round(&store, &actor, &scope);
}

#[test]
#[ignore = "release control query CPU comparison; not end-to-end or native acceptance"]
fn measure_control_query_compilation_cost() {
    let mut store = ControlStore::open_in_memory().unwrap();
    let (actor, scope) = grant_fixture(&mut store);
    let mut rounds = Vec::new();
    // 同进程 AB/BA，容量为零只禁用编译缓存；每轮仍执行同样 SQL/实时断言/期限检查。
    for capacity in [0, 16, 16, 0] {
        store.connection.flush_prepared_statement_cache();
        store
            .connection
            .set_prepared_statement_cache_capacity(capacity);
        let mut samples = Vec::new();
        for _ in 0..2000 {
            let start = Instant::now();
            store
                .with_read_deadline(
                    start + Duration::from_millis(50),
                    |store| -> crate::Result<()> {
                        read_round(store, &actor, &scope);
                        Ok(())
                    },
                )
                .unwrap();
            samples.push(start.elapsed().as_nanos() as u64);
        }
        samples.sort_unstable();
        rounds.push(
            serde_json::json!({"capacity":capacity,"samples":samples.len(),
            "p50_ns":samples[999],"p95_ns":samples[1899],"total_ns":samples.iter().sum::<u64>()}),
        );
    }
    println!(
        "{}",
        serde_json::json!({"scope":"in-memory control SQL compilation only",
        "os":std::env::consts::OS,"release":!cfg!(debug_assertions),"rounds":rounds})
    );
}
