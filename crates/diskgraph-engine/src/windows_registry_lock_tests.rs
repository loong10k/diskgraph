//! Registry 预取锁竞争仅验证准入；不冒充原生进程清理资格。
use super::ScanWorkerRegistry;
use std::time::{Duration, Instant};

#[test]
fn contended_registry_drain_returns_pending_without_taking_reserved_slot() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let reservation = registry.reserve().unwrap();
    let slots = registry.slots.lock().unwrap();
    let deadline = Instant::now() + Duration::from_millis(100);
    assert!(!registry.drain_until(deadline).unwrap());
    assert!(!registry.drain_until(Instant::now()).unwrap());
    assert!(matches!(
        slots[0],
        crate::scan_worker_owner_slot::ScanWorkerOwnerSlot::Reserved
    ));
    drop(slots);
    assert_eq!(registry.occupied().unwrap(), 1);
    drop(reservation);
    assert_eq!(registry.occupied().unwrap(), 0);
}
