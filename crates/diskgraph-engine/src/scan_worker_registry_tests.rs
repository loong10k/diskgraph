//! 有限槽的出生前准入验收；来源：PF-06。
//! 此处没有创建OS child，只验证预留/未出生释放，不伪造reaped或失败owner。

use crate::EngineError;
use crate::scan_worker_registry::ScanWorkerRegistry;
use diskgraph_core::BusinessError;
use std::sync::{Arc, Barrier};

#[test]
fn slot_is_reserved_before_birth_and_capacity_never_overbooks() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let first = registry.reserve().unwrap();
    assert_eq!(registry.occupied().unwrap(), 1);
    assert!(matches!(
        registry.reserve(),
        Err(EngineError::Business(BusinessError::ResourceExhausted))
    ));
    drop(first);
    assert_eq!(registry.occupied().unwrap(), 0);
    let next = registry.reserve().unwrap();
    assert_eq!(registry.occupied().unwrap(), 1);
    drop(next);
}

#[test]
fn prebirth_unwind_releases_only_the_unborn_reservation() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let sentinel = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _reservation = registry.reserve().unwrap();
        assert_eq!(registry.occupied().unwrap(), 1);
        std::panic::panic_any("prebirth-sentinel");
    }))
    .unwrap_err();
    assert_eq!(sentinel.downcast_ref::<&str>(), Some(&"prebirth-sentinel"));
    assert_eq!(registry.occupied().unwrap(), 0);
}

#[test]
fn simultaneous_prebirth_requests_share_one_original_capacity() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let ready = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    std::thread::scope(|threads| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let registry = Arc::clone(&registry);
            let ready = Arc::clone(&ready);
            let release = Arc::clone(&release);
            handles.push(threads.spawn(move || {
                let reservation = registry.reserve();
                let accepted = reservation.is_ok();
                let denied = matches!(
                    &reservation,
                    Err(EngineError::Business(BusinessError::ResourceExhausted))
                );
                ready.wait();
                release.wait();
                drop(reservation);
                (accepted, denied)
            }));
        }
        ready.wait();
        let occupied = registry.occupied();
        release.wait();
        let observations = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(occupied.unwrap(), 1);
        assert_eq!(
            observations
                .iter()
                .filter(|(accepted, _)| *accepted)
                .count(),
            1
        );
        assert_eq!(observations.iter().filter(|(_, denied)| *denied).count(), 1);
    });
    assert_eq!(registry.occupied().unwrap(), 0);
}

#[test]
#[cfg(unix)]
fn scan_worker_ownership_moves_do_not_copy_inline_read_buffers() {
    // 固定容量表和失败返回反复移动 owner，读缓冲不应放大每个空槽及错误栈。
    let slot = std::mem::size_of::<crate::scan_worker_owner_slot::ScanWorkerOwnerSlot>();
    let failure = std::mem::size_of::<
        crate::scan_worker_owned_failure::ScanWorkerOwnedFailure<std::convert::Infallible>,
    >();
    assert!(slot <= 256, "owner slot unexpectedly costs {slot} bytes");
    assert!(
        failure <= 256,
        "owned failure unexpectedly costs {failure} bytes"
    );
}
