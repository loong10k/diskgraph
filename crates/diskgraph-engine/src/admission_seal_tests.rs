//! 真实原容量预留的关闭回归；未创建 child，不将空池证明代替原生回收。
#[cfg(windows)]
use crate::ProbeRecovery;
#[cfg(windows)]
use crate::probe_resource_pool::ProbeResourcePool;
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{EngineError, ScanWorkerRecovery};
use diskgraph_core::BusinessError;
use std::sync::{Arc, Barrier};
fn is_closed<T>(result: Result<T, EngineError>) -> bool {
    matches!(result, Err(EngineError::Business(BusinessError::Conflict)))
}

#[test]
fn original_scan_reservation_survives_seal_and_new_births_are_refused() {
    let pool = ScanWorkerRegistry::new(1).unwrap();
    let recovery = ScanWorkerRecovery::new(Arc::clone(&pool));
    let original = pool.reserve().unwrap();
    recovery.seal_admission().unwrap();
    recovery.seal_admission().unwrap();
    assert_eq!(recovery.occupied_slots().unwrap(), 1);
    drop(original);
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(is_closed(pool.reserve()));
    assert!(recovery.drain().unwrap());
    assert!(is_closed(pool.reserve()));
}
#[cfg(windows)]
#[test]
fn original_probe_session_survives_seal_and_reuse_is_refused() {
    let pool = ProbeResourcePool::new(1).unwrap();
    let recovery = ProbeRecovery::new(Arc::clone(&pool));
    let original = pool.reserve().unwrap();
    recovery.seal_admission().unwrap();
    recovery.seal_admission().unwrap();
    assert_eq!(recovery.occupied_slots().unwrap(), 1);
    assert!(!recovery.drain().unwrap());
    drop(original);
    assert!(recovery.drain().unwrap());
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(is_closed(pool.reserve()));
}
#[test]
fn concurrent_scan_reserve_cannot_cross_completed_seal() {
    let pool = ScanWorkerRegistry::new(16).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    std::thread::scope(|scope| {
        let p = Arc::clone(&pool);
        let b = Arc::clone(&barrier);
        scope.spawn(move || {
            b.wait();
            p.seal_admission().unwrap();
        });
        let p = Arc::clone(&pool);
        let b = Arc::clone(&barrier);
        scope.spawn(move || {
            b.wait();
            for _ in 0..1000 {
                drop(p.reserve());
            }
        });
    });
    assert_eq!(pool.occupied().unwrap(), 0);
    for _ in 0..100 {
        assert!(is_closed(pool.reserve()));
    }
}
