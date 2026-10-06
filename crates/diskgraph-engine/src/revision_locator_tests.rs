//! 实际扫描节点的首末授权、旧定位拒绝和读取预算验收。

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use crate::Engine;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{BusinessError, Permission, PrincipalId, QueryBudget, ScopeId};
use diskgraph_store::{ControlStore, StoreError};
use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

type AfterRead = Box<dyn FnOnce(Instant)>;
thread_local! {
    static AFTER_READ: RefCell<Option<AfterRead>> = RefCell::new(None);
}

pub(super) fn after_read(deadline: Instant) {
    if let Some(callback) = AFTER_READ.with(|slot| slot.borrow_mut().take()) {
        callback(deadline);
    }
}

fn fixture() -> (tempfile::TempDir, Engine, PrincipalId, ScopeId, String, u64) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("文件.txt"), b"payload").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("locator-reader").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "locator-reader").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let node = engine
        .revision_node_at(&revision, Path::new("文件.txt"))
        .unwrap()
        .unwrap()
        .id;
    (dir, engine, actor, scope, revision, node)
}

#[test]
fn qualified_locator_reads_actual_published_node_without_display_addressing() {
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let stored = engine
        .revision_node_locator(
            &revision,
            node,
            &actor,
            &engine.policy_authorizer().unwrap(),
            QueryBudget::default(),
        )
        .unwrap();
    assert_eq!(
        stored.locator.unwrap().to_native_path().unwrap(),
        dir.path().join("root/文件.txt").canonicalize().unwrap()
    );
    assert!(stored.self_modified.is_some());
}

#[test]
fn qualified_locator_missing_node_is_not_an_unavailable_legacy_node() {
    let (_dir, engine, actor, _scope, revision, _node) = fixture();
    let result = engine.revision_node_locator(
        &revision,
        u64::MAX,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Store(StoreError::IntegerOverflow))
    ));
    let result = engine.revision_node_locator(
        &revision,
        1_000_000,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::NotFound))
    ));
}

#[test]
fn qualified_locator_legacy_row_requires_reindex_instead_of_display_fallback() {
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute("UPDATE nodes SET native_locator_kind=NULL,native_locator_encoding=NULL,native_locator_raw=NULL,self_modified_unix_seconds=NULL WHERE id=?1", [i64::try_from(node).unwrap()]).unwrap();
    let result = engine.revision_node_locator(
        &revision,
        node,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Unsupported))
    ));
    assert_eq!(
        engine.revision_node(&revision, node).unwrap().id,
        node,
        "old display query remains available"
    );
}

#[test]
fn qualified_locator_denies_ungranted_subject_before_decoding_corrupt_raw() {
    let (dir, engine, _actor, _scope, revision, node) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute("UPDATE nodes SET native_locator_encoding='unknown-encoding',native_locator_raw=x'00' WHERE id=?1", [i64::try_from(node).unwrap()]).unwrap();
    let stranger = PrincipalId::new("ungranted-locator").unwrap();
    let result = engine.revision_node_locator(
        &revision,
        node,
        &stranger,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn qualified_locator_scope_a_permission_cannot_read_scope_b_revision() {
    let (dir, engine, actor, scope, _revision, _node) = fixture();
    let other = dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let policy = engine.policy_authorizer().unwrap();
    let other_scope = engine.register_scope(&other, &actor, &policy).unwrap();
    let job = engine
        .index_scope(&other_scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "other-locator").unwrap();
    let revision = engine.latest_revision(&other_scope).unwrap().unwrap();
    let reader = PrincipalId::new("only-scope-a").unwrap();
    let mut control = engine.control_store().unwrap();
    let version = control.policy_state().unwrap().unwrap().0;
    control
        .upsert_grant(&diskgraph_core::Grant {
            principal: reader.clone(),
            permission: Permission::MetadataRead,
            scope,
            policy_version: version,
        })
        .unwrap();
    drop(control);
    let result = engine.revision_node_locator(
        &revision,
        1,
        &reader,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn qualified_locator_large_raw_refuses_before_invalid_identity_decode() {
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute(
        "UPDATE nodes SET native_locator_raw=zeroblob(200000) WHERE id=?1",
        [i64::try_from(node).unwrap()],
    )
    .unwrap();
    let result = engine.revision_node_locator(
        &revision,
        node,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget {
            max_response_bytes: 1024,
            ..QueryBudget::default()
        },
    );
    assert!(matches!(
        result,
        Err(EngineError::Store(StoreError::BudgetExceeded))
    ));
}

fn terminal_revocation(scope_revoked: bool) {
    let (dir, engine, actor, scope, revision, node) = fixture();
    let path = dir.path().join("data/diskgraph-control.sqlite");
    let actor_copy = actor.clone();
    AFTER_READ.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |_| {
            let mut control = ControlStore::open(&path).unwrap();
            if scope_revoked {
                control.revoke_scope(&scope).unwrap();
            } else {
                control
                    .revoke_grant(&actor_copy, &Permission::MetadataRead, &scope)
                    .unwrap();
            }
        }))
    });
    let result = engine.revision_node_locator(
        &revision,
        node,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(
        AFTER_READ.with(|slot| slot.borrow().is_none()),
        "actual read hook was not reached"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "returned locator after revocation: {result:?}"
    );
}

#[test]
fn qualified_locator_refuses_terminal_scope_revocation() {
    terminal_revocation(true);
}

#[test]
fn qualified_locator_refuses_terminal_metadata_grant_revocation() {
    terminal_revocation(false);
}

#[test]
fn qualified_locator_preserves_original_deadline_through_terminal_authorization() {
    let (_dir, engine, actor, _scope, revision, node) = fixture();
    AFTER_READ.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |deadline| {
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(10),
            );
        }))
    });
    let result = engine.revision_node_locator(
        &revision,
        node,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(AFTER_READ.with(|slot| slot.borrow().is_none()));
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}
