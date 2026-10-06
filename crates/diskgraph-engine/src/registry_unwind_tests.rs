//! 真实原进程清理展开时必须归还原槽；来源：PF-06 原 owner 恢复合同。
use super::ScanWorkerRegistry;
use crate::native_child::UnixChild;
use crate::scan_worker_owner_slot::ScanWorkerOwnerSlot;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::Command;

thread_local! {
    static PANIC_BEFORE_CLEANUP: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn cleanup_checkpoint() {
    if PANIC_BEFORE_CLEANUP.replace(false) {
        std::panic::panic_any("original-registry-cleanup-panic");
    }
}

#[test]
fn cleanup_unwind_returns_live_original_owner_to_same_slot() {
    for bounded in [false, true] {
        verify_cleanup_unwind_returns_original(bounded);
    }
}

fn verify_cleanup_unwind_returns_original(bounded: bool) {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let reservation = registry.reserve().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("60");
    let child = UnixChild::spawn_checked(&mut command, || Ok::<(), ()>(())).unwrap();
    reservation.retain(child);
    drop(reservation);
    PANIC_BEFORE_CLEANUP.set(true);
    let payload = catch_unwind(AssertUnwindSafe(|| {
        if bounded {
            registry.drain_until(std::time::Instant::now() + std::time::Duration::from_secs(10))
        } else {
            registry.drain()
        }
    }))
    .unwrap_err();
    let retained_and_live = {
        let mut slots = registry.slots.lock().unwrap();
        match &mut slots[0] {
            ScanWorkerOwnerSlot::Retained(owner) => !owner.poll().unwrap(),
            _ => false,
        }
    };
    let still_full = registry.reserve().is_err();
    // 在断言前处置原进程；旧实现已经在展开中 Drop 它并留下永久 Draining 槽。
    let actually_drained = registry.drain().unwrap();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"original-registry-cleanup-panic")
    );
    assert!(
        retained_and_live,
        "unwind lost the live original owner instead of returning it to its slot"
    );
    assert!(still_full, "unreaped child released capacity");
    assert!(
        actually_drained,
        "original child was not actually recovered"
    );
    assert_eq!(registry.occupied().unwrap(), 0);
}

#[test]
fn cleanup_error_keeps_original_capacity_until_actual_retry() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let reservation = registry.reserve().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("60").env("DG_PROBE_CLEANUP_FAULT", "1");
    let child = UnixChild::spawn_checked(&mut command, || Ok::<(), ()>(())).unwrap();
    reservation.retain(child);
    drop(reservation);
    let error = registry.drain().unwrap_err();
    let occupied = registry.occupied().unwrap();
    let denied = registry.reserve().is_err();
    let live = {
        let mut slots = registry.slots.lock().unwrap();
        match &mut slots[0] {
            ScanWorkerOwnerSlot::Retained(owner) => !owner.poll().unwrap(),
            _ => false,
        }
    };
    // 在验证主错误和容量前，由同一 registry 实际清理原 owner。
    let recovered = registry.drain().unwrap();
    assert!(
        error.to_string().contains("injected cleanup failure"),
        "{error}"
    );
    assert_eq!(occupied, 1);
    assert!(denied && live);
    assert!(recovered);
    assert_eq!(registry.occupied().unwrap(), 0);
}

#[test]
fn deadline_drain_refuses_contended_state_lock_without_consuming_original_owner() {
    use std::time::{Duration, Instant};
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let reservation = registry.reserve().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("60");
    reservation.retain(UnixChild::spawn_checked(&mut command, || Ok::<(), ()>(())).unwrap());
    drop(reservation);
    let locked = std::sync::Arc::clone(&registry);
    let (ready, received) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = locked.slots.lock().unwrap();
        ready.send(()).unwrap();
        // 固定释放保证故障实现即使阻塞也有界收场，不依赖主调用来释放自身等待的锁。
        std::thread::sleep(Duration::from_millis(600));
    });
    received.recv().unwrap();
    let started = Instant::now();
    let result = registry.drain_until(started + Duration::from_secs(10));
    let elapsed = started.elapsed();
    holder.join().unwrap();
    let retained = registry.occupied().unwrap();
    let refused = registry.reserve().is_err();
    let completed = registry.drain().unwrap();
    assert!(!result.unwrap());
    assert!(elapsed < Duration::from_millis(400));
    assert_eq!(retained, 1);
    assert!(refused && completed);
}
