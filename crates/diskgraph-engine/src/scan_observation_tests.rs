//! 本次扫描的原生补充观测必须经暂存实际发布，不能仅创建新列。

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::Engine;
use crate::EngineConfig;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::native_scan_engine_fixture::NativeScanEngine as Engine;
use diskgraph_core::PrincipalId;
#[cfg(not(windows))]
use diskgraph_core::{QueryBudget, QueryReadBudget, WindowsObservationGap};
use std::cell::RefCell;

type CaptureHook = (String, Box<dyn FnOnce()>);
thread_local! { static AFTER_OBSERVE: RefCell<Option<CaptureHook>> = RefCell::new(None); }
thread_local! { static BEFORE_STAGE_LOCK: RefCell<Option<CaptureHook>> = RefCell::new(None); }

pub(super) fn before_stage_lock(job_id: &str) {
    let callback = BEFORE_STAGE_LOCK.with(|slot| {
        if slot.borrow().as_ref().is_some_and(|(job, _)| job == job_id) {
            slot.borrow_mut().take().map(|(_, callback)| callback)
        } else {
            None
        }
    });
    if let Some(callback) = callback {
        callback();
    }
}

pub(super) fn after_observe(job_id: &str) {
    let callback = AFTER_OBSERVE.with(|slot| {
        if slot.borrow().as_ref().is_some_and(|(job, _)| job == job_id) {
            slot.borrow_mut().take().map(|(_, callback)| callback)
        } else {
            None
        }
    });
    if let Some(callback) = callback {
        callback();
    }
}

fn fixture() -> (
    tempfile::TempDir,
    Engine,
    PrincipalId,
    diskgraph_core::ScopeId,
    diskgraph_store::JobRecord,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"payload").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("observation-guard").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    (dir, engine, actor, scope, job)
}

#[cfg(windows)]
#[test]
fn windows_native_observation_persists_real_file_and_hardlink_identity_after_reopen() {
    use diskgraph_core::{QueryBudget, WindowsTreeAlignment};
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES,
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let file = root.join("file");
    std::fs::write(&file, b"payload").unwrap();
    std::fs::hard_link(&file, root.join("link")).unwrap();
    let mut config = EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    };
    config.scan_options.apparent_size = true;
    config.scan_options.dedup_hardlinks = false;
    let engine = Engine::open(config.clone()).unwrap();
    let actor = PrincipalId::new("native-observation-persist").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine
        .run_job(&job.job_id, "native-observation-persist")
        .unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    drop(engine);
    let engine = Engine::open(config).unwrap();
    let mut observed = Vec::new();
    for name in ["file", "link"] {
        let node = engine
            .revision_node_at(&revision, std::path::Path::new(name))
            .unwrap()
            .unwrap();
        let value = engine
            .revision_windows_observation(
                &revision,
                node.id,
                &actor,
                &engine.policy_authorizer().unwrap(),
                QueryBudget::default(),
            )
            .unwrap();
        assert_eq!(value.gap, None);
        let value = value.observation.unwrap();
        assert_eq!(value.length, 7);
        assert_eq!(value.tree_alignment, WindowsTreeAlignment::Matched);
        let handle = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .custom_flags(
                FILE_FLAG_OPEN_REPARSE_POINT
                    | FILE_FLAG_BACKUP_SEMANTICS
                    | FILE_FLAG_OPEN_NO_RECALL,
            )
            .open(root.join(name))
            .unwrap();
        let expected = crate::windows_file_state::WindowsFileState::capture(&handle)
            .unwrap()
            .observation(
                value.capture_started_unix_ms,
                value.capture_finished_unix_ms,
                value.tree_alignment,
            );
        assert_eq!(
            value, expected,
            "published attributes must come from the actual native object"
        );
        observed.push(value);
    }
    assert_eq!(observed[0].volume, observed[1].volume);
    assert_eq!(observed[0].file_id, observed[1].file_id);
}

#[test]
fn observation_stage_refuses_grant_scope_and_fence_changes_after_native_capture() {
    for mode in 0..3 {
        let (dir, engine, actor, scope, job) = fixture();
        let control_path = dir.path().join("data/diskgraph-control.sqlite");
        // 外部管理员连接先准备完成。目标竞态只发生在实际采样后的授权/fence修改，
        // 不把连接初始化与keeper检查的锁竞争当成已经执行撤权。
        let mut control = diskgraph_store::ControlStore::open(&control_path).unwrap();
        let fence_connection = rusqlite::Connection::open(&control_path).unwrap();
        let job_id = job.job_id.clone();
        AFTER_OBSERVE.with(|slot| {
            *slot.borrow_mut() = Some((
                job.job_id.clone(),
                Box::new(move || match mode {
                    0 => {
                        control.revoke_scope(&scope).unwrap();
                    }
                    1 => {
                        control
                            .revoke_grant(&actor, &diskgraph_core::Permission::IndexWrite, &scope)
                            .unwrap();
                    }
                    _ => {
                        fence_connection
                            .execute(
                                "UPDATE jobs SET fencing_token=fencing_token+1 WHERE job_id=?1",
                                [&job_id],
                            )
                            .unwrap();
                    }
                }),
            ))
        });
        assert!(engine.run_job(&job.job_id, "observation-guard").is_err());
        assert!(
            AFTER_OBSERVE.with(|slot| slot.borrow().is_none()),
            "real capture hook not reached"
        );
        let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
        for table in [
            "scan_staging",
            "snapshots",
            "graph_revisions",
            "collector_runs",
        ] {
            let count: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "mode {mode}: {table}");
        }
    }
}

#[test]
fn observation_guard_checks_fast_cancellation_and_original_scan_clock() {
    use crate::EngineError;
    use crate::scan_observation_guard::ScanObservationGuard;
    use diskgraph_core::BusinessError;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_dir, engine, _actor, _scope, job) = fixture();
    let job = engine
        .control_store()
        .unwrap()
        .claim_job_once(&job.job_id, "guard-clock")
        .unwrap();
    let cancel = AtomicBool::new(false);
    let authority = engine
        .control_store()
        .unwrap()
        .job_request_authority(&job.job_id)
        .unwrap();
    let guard = ScanObservationGuard::new(
        &engine,
        &job,
        authority.as_ref(),
        &cancel,
        std::time::Instant::now(),
    );
    guard.check_now().unwrap();
    cancel.store(true, Ordering::SeqCst);
    assert!(matches!(
        guard.check(),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    cancel.store(false, Ordering::SeqCst);
    let start = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_millis(
            engine.scan_budget.max_duration_ms + 1,
        ))
        .unwrap();
    let guard = ScanObservationGuard::new(&engine, &job, authority.as_ref(), &cancel, start);
    assert!(matches!(
        guard.check(),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn staging_lock_wait_cannot_spend_expired_or_cancelled_scan_budget() {
    use std::sync::{Arc, atomic::Ordering};
    for expire in [true, false] {
        let (dir, mut engine, _actor, _scope, job) = fixture();
        engine.scan_budget.max_duration_ms = if expire { 5000 } else { 10000 };
        let engine = Arc::new(engine);
        let worker_engine = Arc::clone(&engine);
        let job_id = job.job_id.clone();
        let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE stage_seen(id INTEGER); CREATE TRIGGER remember_stage AFTER INSERT ON scan_staging BEGIN INSERT INTO stage_seen VALUES(NEW.node_seq); END;").unwrap();
        BEFORE_STAGE_LOCK.with(|slot| {
            *slot.borrow_mut() = Some((
                job.job_id.clone(),
                Box::new(move || {
                    let (sender, receiver) = std::sync::mpsc::sync_channel(0);
                    let (start, started) = std::sync::mpsc::sync_channel(0);
                    let observer = Arc::clone(&worker_engine);
                    std::thread::spawn(move || {
                        let held = worker_engine.graph().unwrap();
                        sender.send(()).unwrap();
                        started
                            .recv_timeout(std::time::Duration::from_secs(10))
                            .unwrap();
                        if expire {
                            std::thread::sleep(std::time::Duration::from_millis(5100));
                        } else {
                            worker_engine
                                .cancellations()
                                .unwrap()
                                .get(&job_id)
                                .unwrap()
                                .store(true, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                        drop(held);
                    });
                    receiver
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    assert!(matches!(
                        observer.graph.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    start.send(()).unwrap();
                }),
            ))
        });
        let outcome = engine.run_job(&job.job_id, "stage-lock-wait");
        assert!(outcome.is_err(), "expire={expire}");
        assert!(
            BEFORE_STAGE_LOCK.with(|slot| slot.borrow().is_none()),
            "did not reach actual staging graph lock"
        );
        let writes: i64 = db
            .query_row("SELECT COUNT(*) FROM stage_seen", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            writes, 0,
            "expire={expire}: staged after waiting past a deadline/cancellation"
        );
        let published: i64 = db
            .query_row("SELECT COUNT(*) FROM graph_revisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(published, 0);
    }
}

#[cfg(not(windows))]
#[test]
fn scan_budget_charges_gap_bytes_in_addition_to_qualified_locator_cost() {
    use crate::{EngineError, scan_node_locator::qualify_scan_locator};
    use diskgraph_core::BusinessError;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"payload").unwrap();
    let mut config = EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    };
    let observed = diskgraph_disktree::scan_native_v2(&root, config.scan_options.clone()).unwrap();
    config.scan_budget.max_staging_bytes = observed
        .nodes
        .iter()
        .map(|node| {
            diskgraph_store::staging_node_encoded_cost(
                &node.v1,
                Some(&qualify_scan_locator(node).unwrap()),
                node.self_modified,
            )
            .unwrap()
        })
        .sum();
    let engine = Engine::open(config).unwrap();
    let actor = PrincipalId::new("observation-budget").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert!(matches!(
        engine.run_job(&job.job_id, "observation-budget"),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert!(engine.latest_revision(&scope).unwrap().is_none());
}

#[cfg(not(windows))]
#[test]
fn non_windows_scan_publishes_explicit_unsupported_native_observation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), b"payload").unwrap();
    let engine = Engine::open(EngineConfig {
        data_dir: dir.path().join("data"),
        ..EngineConfig::default()
    })
    .unwrap();
    let actor = PrincipalId::new("observation-scan").unwrap();
    engine.bootstrap_local_admin(&actor).unwrap();
    let scope = engine
        .register_scope(&root, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    let job = engine
        .index_scope(&scope, &actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    engine.run_job(&job.job_id, "observation-scan").unwrap();
    let revision = engine.latest_revision(&scope).unwrap().unwrap();
    let reader = engine.revision_reader().unwrap();
    let snapshot = reader.revision(&revision).unwrap().snapshot_id;
    let graph = reader.load_revision(&revision).unwrap();
    for node in graph.nodes {
        let budget = QueryBudget::default();
        let mut reads = QueryReadBudget::new(
            budget,
            std::time::Instant::now() + std::time::Duration::from_millis(budget.deadline_ms),
        )
        .unwrap();
        let stored = reader
            .windows_observation_bounded(&snapshot, node.id, &mut reads)
            .unwrap()
            .unwrap();
        assert_eq!(stored.observation, None);
        assert_eq!(
            stored.gap,
            Some(WindowsObservationGap::Unsupported),
            "new scan must distinguish unsupported host from uncaptured legacy data"
        );
    }
}

#[cfg(target_os = "linux")]
mod linux_namespace_tests;
