//! 实际扫描节点的首末授权、旧定位拒绝和读取预算验收。

#[cfg(not(target_os = "linux"))]
use crate::Engine;
#[cfg(target_os = "linux")]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use crate::{EngineConfig, EngineError};
use diskgraph_core::{BusinessError, Permission, PrincipalId, QueryBudget, ScopeId};
use diskgraph_store::{ControlStore, StoreError};
use std::cell::RefCell;
use std::path::Path;
use std::time::{Duration, Instant};

type AfterRead = Box<dyn FnOnce(Instant)>;

#[test]
fn complete_windows_metadata_can_be_read_on_a_foreign_host_without_path_conversion() {
    use diskgraph_core::{WindowsFileObservation, WindowsTreeAlignment};
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let observation = WindowsFileObservation {
        volume: u64::MAX,
        file_id: [255; 16],
        length: 7,
        creation_time: i64::MIN,
        last_write_time: 13_400_000_000_000_001,
        change_time: i64::MAX,
        attributes: 0x20,
        directory: false,
        delete_pending: false,
        capture_started_unix_ms: 10,
        capture_finished_unix_ms: 11,
        tree_alignment: WindowsTreeAlignment::Unverified,
    };
    let raw = observation.encode().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute("UPDATE nodes SET native_locator_kind='native_path',native_locator_encoding='windows_utf16_le',native_observation_format=?1,native_observation_raw=?2,native_observation_gap=NULL WHERE id=?3", rusqlite::params![WindowsFileObservation::FORMAT_LABEL,raw.as_slice(),i64::try_from(node).unwrap()]).unwrap();
    let value = engine
        .revision_windows_observation(
            &revision,
            node,
            &actor,
            &engine.policy_authorizer().unwrap(),
            QueryBudget::default(),
        )
        .unwrap();
    assert_eq!(value.observation, Some(observation));
    assert_eq!(value.gap, None);
}
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
fn windows_observation_reads_actual_published_node_as_metadata() {
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let stored = engine
        .revision_windows_observation(
            &revision,
            node,
            &actor,
            &engine.policy_authorizer().unwrap(),
            QueryBudget::default(),
        )
        .unwrap();
    #[cfg(not(windows))]
    assert_eq!(
        stored.gap,
        Some(diskgraph_core::WindowsObservationGap::Unsupported)
    );
    #[cfg(windows)]
    assert!(stored.observation.is_some());
    assert!(dir.path().exists());
}

#[test]
fn windows_observation_missing_node_is_not_an_unavailable_legacy_node() {
    let (_dir, engine, actor, _scope, revision, _node) = fixture();
    let result = engine.revision_windows_observation(
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
    let result = engine.revision_windows_observation(
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
fn windows_observation_legacy_row_reports_not_captured_without_inference() {
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute("UPDATE nodes SET native_observation_format=NULL,native_observation_raw=NULL,native_observation_gap=NULL WHERE id=?1", [i64::try_from(node).unwrap()]).unwrap();
    let result = engine.revision_windows_observation(
        &revision,
        node,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert_eq!(
        result.unwrap().gap,
        Some(diskgraph_core::WindowsObservationGap::NotCaptured)
    );
    assert_eq!(
        engine.revision_node(&revision, node).unwrap().id,
        node,
        "old display query remains available"
    );
}

#[test]
fn windows_observation_denies_ungranted_subject_before_decoding_corrupt_raw() {
    let (dir, engine, _actor, _scope, revision, node) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute("UPDATE nodes SET native_observation_format='unknown',native_observation_raw=x'00',native_observation_gap=NULL WHERE id=?1", [i64::try_from(node).unwrap()]).unwrap();
    let stranger = PrincipalId::new("ungranted-locator").unwrap();
    let result = engine.revision_windows_observation(
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
fn windows_observation_scope_a_permission_cannot_read_scope_b_revision() {
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
    let result = engine.revision_windows_observation(
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
fn windows_observation_large_raw_refuses_before_invalid_identity_decode() {
    let (dir, engine, actor, _scope, revision, node) = fixture();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute(
        "UPDATE nodes SET native_observation_format='windows_file_observation_v1',native_observation_gap=NULL,native_observation_raw=zeroblob(200000) WHERE id=?1",
        [i64::try_from(node).unwrap()],
    )
    .unwrap();
    let result = engine.revision_windows_observation(
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
    let result = engine.revision_windows_observation(
        &revision,
        node,
        &actor,
        &engine.policy_authorizer().unwrap(),
        QueryBudget::default(),
    );
    assert!(
        AFTER_READ.with(|slot| slot.borrow().is_none()),
        "actual observation read hook was not reached"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "returned observation after revocation: {result:?}"
    );
}

#[test]
fn windows_observation_refuses_terminal_scope_revocation() {
    terminal_revocation(true);
}

#[test]
fn windows_observation_refuses_terminal_metadata_grant_revocation() {
    terminal_revocation(false);
}

#[test]
fn windows_observation_preserves_original_deadline_through_terminal_authorization() {
    let (_dir, engine, actor, _scope, revision, node) = fixture();
    AFTER_READ.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |deadline| {
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(10),
            );
        }))
    });
    let result = engine.revision_windows_observation(
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
