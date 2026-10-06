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
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let reservation = registry.reserve().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("60");
    let child = UnixChild::spawn_checked(&mut command, || Ok::<(), ()>(())).unwrap();
    reservation.retain(child);
    drop(reservation);
    PANIC_BEFORE_CLEANUP.set(true);
    let payload = catch_unwind(AssertUnwindSafe(|| registry.drain())).unwrap_err();
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
