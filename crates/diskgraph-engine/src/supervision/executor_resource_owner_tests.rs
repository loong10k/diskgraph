//! 原executor资源与guardian容量分离的真实隔离夹具，不出生OS进程。
use crate::recovery_slot::{SlotError, SlotReservation};
use crate::{ExecutorParts, ExecutorResourceOwner};
use std::fs::OpenOptions;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[test]
fn actual_pending_reservation_prevents_resource_retirement_without_slot_access() {
    let (directory, parts, registry) = crate::supervisor_binding_tests::fixture();
    let crate::SupervisorParts {
        engine,
        scan,
        slot: mut guardian_slot,
        #[cfg(windows)]
        probe,
    } = parts;
    let pending = registry.reserve().unwrap();
    let weak_engine = Arc::downgrade(&engine);
    let mut executor = ExecutorResourceOwner::bind(ExecutorParts {
        engine,
        scan,
        #[cfg(windows)]
        probe,
    })
    .unwrap_or_else(|_| panic!("original executor binding rejected"));
    assert!(!executor.poll_retirement(deadline()).unwrap());
    assert!(weak_engine.upgrade().is_none());
    assert_eq!(registry.occupied().unwrap(), 1);
    guardian_slot.verify_active(deadline()).unwrap();
    drop(pending);
    assert!(executor.poll_retirement(deadline()).unwrap());
    assert!(executor.poll_retirement(deadline()).unwrap());
    // executor资源完成不清槽；没有实际executor进程wait的本夹具保持ACTIVE。
    guardian_slot.verify_active(deadline()).unwrap();
    let foreign = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join("slot"))
        .unwrap();
    assert!(matches!(
        SlotReservation::acquire(foreign, deadline()),
        Err(SlotError::Busy)
    ));
}

#[test]
fn executor_external_engine_reference_retains_original_owner() {
    let (_directory, parts, registry) = crate::supervisor_binding_tests::fixture();
    let crate::SupervisorParts {
        engine,
        scan,
        slot: mut guardian_slot,
        #[cfg(windows)]
        probe,
    } = parts;
    let original = Arc::clone(&engine);
    let mut executor = ExecutorResourceOwner::bind(ExecutorParts {
        engine,
        scan,
        #[cfg(windows)]
        probe,
    })
    .unwrap_or_else(|_| panic!("original executor binding rejected"));
    assert!(!executor.poll_retirement(deadline()).unwrap());
    assert!(Arc::ptr_eq(executor.engine().unwrap(), &original));
    assert!(matches!(
        registry.reserve(),
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::Conflict
        ))
    ));
    guardian_slot.verify_active(deadline()).unwrap();
    drop(original);
    assert!(executor.poll_retirement(deadline()).unwrap());
    guardian_slot.verify_active(deadline()).unwrap();
}

#[test]
fn foreign_executor_recovery_returns_both_original_pools_without_sealing() {
    let (_directory, parts, original_registry) = crate::supervisor_binding_tests::fixture();
    let crate::SupervisorParts {
        engine,
        scan,
        slot: mut guardian_slot,
        #[cfg(windows)]
        probe,
    } = parts;
    let original_engine = Arc::clone(&engine);
    let foreign = crate::scan_worker_registry::ScanWorkerRegistry::new(1).unwrap();
    let rejected = match ExecutorResourceOwner::bind(ExecutorParts {
        engine,
        scan: Some(crate::ScanWorkerRecovery::new(Arc::clone(&foreign))),
        #[cfg(windows)]
        probe,
    }) {
        Err(parts) => parts,
        Ok(_) => panic!("foreign recovery bound to executor"),
    };
    assert!(Arc::ptr_eq(&rejected.engine, &original_engine));
    assert!(original_registry.reserve().is_ok());
    assert!(foreign.reserve().is_ok());
    guardian_slot.verify_active(deadline()).unwrap();
    drop(scan);
}

#[test]
fn expired_resource_budget_retains_engine_and_open_original_pool() {
    let (_directory, parts, registry) = crate::supervisor_binding_tests::fixture();
    let crate::SupervisorParts {
        engine,
        scan,
        slot: mut guardian_slot,
        #[cfg(windows)]
        probe,
    } = parts;
    let original_engine = Arc::clone(&engine);
    let mut executor = ExecutorResourceOwner::bind(ExecutorParts {
        engine,
        scan,
        #[cfg(windows)]
        probe,
    })
    .unwrap_or_else(|_| panic!("original executor binding rejected"));
    assert!(matches!(
        executor.poll_retirement(Instant::now() - Duration::from_secs(1)),
        Err(crate::EngineError::Business(
            diskgraph_core::BusinessError::BudgetExceeded
        ))
    ));
    assert!(Arc::ptr_eq(executor.engine().unwrap(), &original_engine));
    assert!(registry.reserve().is_ok());
    guardian_slot.verify_active(deadline()).unwrap();
}
