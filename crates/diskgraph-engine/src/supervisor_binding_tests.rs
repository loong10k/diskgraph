//! 原 Engine/Recovery/槽的绑定与退休；只创建真实未出生容量，不冒充原生 child 回收验收。
use crate::recovery_slot::{SlotError, SlotReservation};
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{
    Engine, EngineConfig, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRecovery,
    ScanWorkerRuntimeBudget, SupervisorOwner, SupervisorParts,
};
use diskgraph_scan_worker::ProtocolLimits;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn open(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
}
/// 参数：无；返回：隔离目录、原监督材料和同一扫描表，不出生实际子进程。
pub(super) fn fixture() -> (tempfile::TempDir, SupervisorParts, Arc<ScanWorkerRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    let slot = SlotReservation::acquire(
        File::create_new(dir.path().join("slot")).unwrap(),
        deadline(),
    )
    .unwrap()
    .activate(deadline())
    .unwrap();
    let bytes = b"independent material; no executable birth in this fixture";
    let image = dir.path().join("image");
    std::fs::write(&image, bytes).unwrap();
    let expected =
        ScanWorkerHostConfig::from_expected_image(Sha256::digest(bytes).into(), bytes.len() as u64)
            .unwrap();
    let host = ScanWorkerHost::new(
        File::open(image).unwrap(),
        expected,
        ScanWorkerRuntimeBudget::new(
            ProtocolLimits {
                max_frame_bytes: 4096,
                max_stream_bytes: 65536,
                max_nodes: 100,
                max_depth: 10,
            },
            0,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    let registry = Arc::clone(&host.registry);
    let (engine, scan) = Engine::open_with_scan_worker(
        EngineConfig {
            data_dir: dir.path().join("data"),
            ..EngineConfig::default()
        },
        host,
    )
    .unwrap();
    (
        dir,
        SupervisorParts {
            engine: Arc::new(engine),
            scan: Some(scan),
            #[cfg(windows)]
            probe: None,
            slot,
        },
        registry,
    )
}
fn bound(parts: SupervisorParts) -> SupervisorOwner {
    SupervisorOwner::bind(parts, deadline()).unwrap_or_else(|_| panic!("original binding refused"))
}
#[test]
fn foreign_recovery_is_rejected_without_consuming_original_materials() {
    let (_dir, mut parts, registry) = fixture();
    let original_engine = Arc::clone(&parts.engine);
    let original = parts.scan.take().unwrap();
    let foreign = ScanWorkerRegistry::new(1).unwrap();
    parts.scan = Some(ScanWorkerRecovery::new(Arc::clone(&foreign)));
    let mut rejected = match SupervisorOwner::bind(parts, deadline()) {
        Err(parts) => parts,
        Ok(_) => panic!("foreign recovery must not bind"),
    };
    assert!(Arc::ptr_eq(&original_engine, &rejected.engine));
    rejected.slot.verify_active(deadline()).unwrap();
    assert!(registry.reserve().is_ok());
    assert!(foreign.reserve().is_ok());
    drop(rejected);
    drop(original);
    drop(original_engine);
}
#[test]
fn missing_recovery_is_rejected_with_original_slot_and_engine_preserved() {
    let (_dir, mut parts, _registry) = fixture();
    let original = parts.scan.take().unwrap();
    let mut rejected = match SupervisorOwner::bind(parts, deadline()) {
        Err(parts) => parts,
        Ok(_) => panic!("missing recovery must not bind"),
    };
    rejected.slot.verify_active(deadline()).unwrap();
    assert!(rejected.engine.scan_worker.is_some());
    drop(rejected);
    drop(original);
}
#[test]
fn actual_original_reservation_keeps_active_capacity_until_release() {
    let (dir, parts, registry) = fixture();
    let reservation = registry.reserve().unwrap();
    let weak = Arc::downgrade(&parts.engine);
    let mut owner = bound(parts);
    assert!(!owner.poll_retirement(deadline()).unwrap());
    assert!(owner.engine().is_none());
    assert!(weak.upgrade().is_none());
    assert_eq!(registry.occupied().unwrap(), 1);
    assert!(matches!(
        registry.reserve(),
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::Conflict
        ))
    ));
    assert!(matches!(
        SlotReservation::acquire(open(&dir.path().join("slot")), deadline()),
        Err(SlotError::Busy)
    ));
    drop(reservation);
    assert!(owner.poll_retirement(deadline()).unwrap());
    assert!(owner.poll_retirement(deadline()).unwrap());
    let next = SlotReservation::acquire(open(&dir.path().join("slot")), deadline()).unwrap();
    assert!(owner.poll_retirement(deadline()).unwrap());
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01R\n"
    );
    let mut next = next.activate(deadline()).unwrap();
    assert!(owner.poll_retirement(deadline()).unwrap());
    next.verify_active(deadline()).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("slot")).unwrap(),
        b"DGSL01A\n"
    );
}
#[test]
fn external_engine_reference_prevents_retirement_and_new_native_birth() {
    let (dir, parts, registry) = fixture();
    let original = Arc::clone(&parts.engine);
    let mut owner = bound(parts);
    assert!(!owner.poll_retirement(deadline()).unwrap());
    assert!(Arc::ptr_eq(owner.engine().unwrap(), &original));
    assert!(matches!(
        registry.reserve(),
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::Conflict
        ))
    ));
    assert!(matches!(
        SlotReservation::acquire(open(&dir.path().join("slot")), deadline()),
        Err(SlotError::Busy)
    ));
    drop(original);
    assert!(owner.poll_retirement(deadline()).unwrap());
}
#[test]
fn expired_retirement_keeps_original_engine_and_active_slot() {
    let (dir, parts, registry) = fixture();
    let mut owner = bound(parts);
    assert!(
        owner
            .poll_retirement(Instant::now() - Duration::from_secs(1))
            .is_err()
    );
    assert!(owner.engine().is_some());
    assert!(registry.reserve().is_ok());
    assert!(matches!(
        SlotReservation::acquire(open(&dir.path().join("slot")), deadline()),
        Err(SlotError::Busy)
    ));
    assert!(owner.poll_retirement(deadline()).unwrap());
}
#[test]
fn dropping_pending_owner_does_not_claim_complete() {
    let (dir, parts, registry) = fixture();
    let original = registry.reserve().unwrap();
    let mut owner = bound(parts);
    assert!(!owner.poll_retirement(deadline()).unwrap());
    drop(owner);
    drop(original);
    let reacquired = SlotReservation::acquire(open(&dir.path().join("slot")), deadline());
    let observed_error = reacquired.as_ref().err();
    assert!(
        matches!(reacquired, Err(SlotError::Unconfirmed)),
        "pending owner must leave unconfirmed capacity; observed error: {observed_error:?}"
    );
}
#[test]
#[cfg(unix)]
fn damaged_record_is_not_repaired_or_released_by_retirement() {
    let (dir, parts, _registry) = fixture();
    let mut owner = bound(parts);
    let path = dir.path().join("slot");
    std::fs::write(&path, b"damaged!").unwrap();
    assert!(owner.poll_retirement(deadline()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"damaged!");
    assert!(matches!(
        SlotReservation::acquire(open(&path), deadline()),
        Err(SlotError::Busy)
    ));
}

#[test]
fn unconfigured_engine_and_extra_recovery_are_not_supervisor_capabilities() {
    let (_dir, mut parts, _registry) = fixture();
    Arc::get_mut(&mut parts.engine).unwrap().scan_worker = None;
    let mut rejected = match SupervisorOwner::bind(parts, deadline()) {
        Err(parts) => parts,
        Ok(_) => panic!("extra recovery must be rejected"),
    };
    rejected.slot.verify_active(deadline()).unwrap();
    drop(rejected.scan.take());
    let mut rejected = match SupervisorOwner::bind(rejected, deadline()) {
        Err(parts) => parts,
        Ok(_) => panic!("unmanaged engine must be rejected"),
    };
    rejected.slot.verify_active(deadline()).unwrap();
}
#[test]
fn original_bind_expiry_returns_every_original_part_without_sealing() {
    let (_dir, parts, registry) = fixture();
    let mut rejected = match SupervisorOwner::bind(parts, Instant::now() - Duration::from_secs(1)) {
        Err(parts) => parts,
        Ok(_) => panic!("expired bind must be refused"),
    };
    rejected.slot.verify_active(deadline()).unwrap();
    assert!(registry.reserve().is_ok());
    assert!(rejected.scan.is_some());
}
#[test]
#[cfg(windows)]
fn original_probe_session_blocks_retirement_until_its_actual_release() {
    let dir = tempfile::tempdir().unwrap();
    let slot = SlotReservation::acquire(
        File::create_new(dir.path().join("slot")).unwrap(),
        deadline(),
    )
    .unwrap()
    .activate(deadline())
    .unwrap();
    let (host, probe) = crate::ProbeHost::new(1).unwrap();
    let registry = Arc::clone(&host.registry);
    let session = registry.reserve().unwrap();
    let (engine, scan) = Engine::open_with_process_hosts(
        EngineConfig {
            data_dir: dir.path().join("data"),
            ..EngineConfig::default()
        },
        None,
        host,
    )
    .unwrap();
    let mut owner = bound(SupervisorParts {
        engine: Arc::new(engine),
        scan,
        probe: Some(probe),
        slot,
    });
    assert!(!owner.poll_retirement(deadline()).unwrap());
    assert_eq!(registry.occupied().unwrap(), 1);
    assert!(matches!(
        registry.reserve(),
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::Conflict
        ))
    ));
    assert!(matches!(
        SlotReservation::acquire(open(&dir.path().join("slot")), deadline()),
        Err(SlotError::Busy)
    ));
    drop(session);
    assert!(owner.poll_retirement(deadline()).unwrap());
    let next = SlotReservation::acquire(open(&dir.path().join("slot")), deadline()).unwrap();
    next.abort_before_birth(deadline()).unwrap();
}

#[test]
#[cfg(windows)]
fn foreign_probe_recovery_is_rejected_and_both_original_pools_remain_usable() {
    let (_dir, mut parts, scan_registry) = fixture();
    let (host, original_probe) = crate::ProbeHost::new(1).unwrap();
    let original_registry = Arc::clone(&host.registry);
    Arc::get_mut(&mut parts.engine).unwrap().probe_host = Some(host);
    let (foreign_host, foreign_probe) = crate::ProbeHost::new(1).unwrap();
    parts.probe = Some(foreign_probe);
    let mut rejected = match SupervisorOwner::bind(parts, deadline()) {
        Err(parts) => parts,
        Ok(_) => panic!("foreign probe must be rejected"),
    };
    rejected.slot.verify_active(deadline()).unwrap();
    assert!(scan_registry.reserve().is_ok());
    assert!(original_registry.reserve().is_ok());
    assert!(foreign_host.registry.reserve().is_ok());
    drop(rejected);
    drop(original_probe);
    drop(foreign_host);
}

#[test]
#[cfg(windows)]
fn scan_pending_does_not_skip_original_probe_cleanup_or_panic() {
    let (directory, mut parts, scan_registry) = fixture();
    let original_scan = scan_registry.reserve().unwrap();
    let (probe, recovery) = crate::ProbeHost::new(1).unwrap();
    let probe_registry = Arc::clone(&probe.registry);
    let session = probe_registry.reserve().unwrap();
    let original_probe = session.inner().reserve().unwrap();
    // 先结束实际会话，再归还原预留；这使同一探针槽进入恢复，而不构造假 owner。
    drop(session);
    drop(original_probe);
    Arc::get_mut(&mut parts.engine).unwrap().probe_host = Some(probe);
    parts.probe = Some(recovery);
    let mut owner = bound(parts);
    crate::probe_pool_cleanup_fault::ProbePoolCleanupFault::arm();
    let until = deadline();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        owner.poll_retirement(until)
    }));
    let original_panic = result.expect_err("pending scan skipped actual original probe recovery");
    assert_eq!(
        original_panic.downcast_ref::<&str>(),
        Some(&"original-probe-pool-cleanup-panic")
    );
    assert!(owner.engine().is_none());
    assert_eq!(scan_registry.occupied().unwrap(), 1);
    assert_eq!(probe_registry.occupied().unwrap(), 1);
    assert!(matches!(
        SlotReservation::acquire(open(&directory.path().join("slot")), until),
        Err(SlotError::Busy)
    ));
    // 不刷新同一轮期限；原 panic 后探针槽保留，下一轮真实释放但扫描仍 Pending。
    assert!(!owner.poll_retirement(until).unwrap());
    assert_eq!(probe_registry.occupied().unwrap(), 0);
    assert_eq!(scan_registry.occupied().unwrap(), 1);
    drop(original_scan);
    assert!(owner.poll_retirement(until).unwrap());
}
