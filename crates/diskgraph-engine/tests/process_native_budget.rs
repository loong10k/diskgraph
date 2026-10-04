//! D42 原生会话纯边界验收；来源：公开原认领时钟/取消/权限回调，不模拟操作系统占用。
use diskgraph_core::{ProcessEvidenceFailureCode as Failure, ProcessEvidenceLimits};
use diskgraph_engine::native_process::ProcessNativeSession;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn native_ledger_shares_remaining_bytes_entries_allocations_and_sticky_failure() {
    let limits = ProcessEvidenceLimits::new(1000, 16, 2, 16, 32, 0, 2).unwrap();
    let cancel = AtomicBool::new(false);
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    session.admit(8, 1, 16).unwrap();
    session.admit(8, 1, 16).unwrap();
    assert_eq!(session.usage(), (16, 2, 32, 0));
    assert_eq!(session.admit(1, 0, 0), Err(Failure::BudgetExceeded));
    assert_eq!(session.admit(0, 0, 0), Err(Failure::BudgetExceeded));
    assert_eq!(session.usage(), (16, 2, 32, 0));
}
#[test]
fn native_session_keeps_original_claim_clock_and_fixed_authorization_failure() {
    let limits = ProcessEvidenceLimits::default();
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        ProcessNativeSession::new(
            &limits,
            Instant::now() - Duration::from_secs(20),
            &cancel,
            &|| Ok(())
        ),
        Err(Failure::Timeout)
    ));
    let allowed = Cell::new(true);
    let check = || {
        if allowed.get() {
            Ok(())
        } else {
            Err(Failure::PermissionDenied)
        }
    };
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &check).unwrap();
    allowed.set(false);
    assert_eq!(session.check(), Err(Failure::PermissionDenied));
    allowed.set(true);
    assert_eq!(session.check(), Err(Failure::PermissionDenied));
}
#[test]
fn cancellation_and_nested_handle_capacity_stop_before_work_and_results_share_limit() {
    let limits = ProcessEvidenceLimits::new(1000, 16, 2, 8, 32, 0, 2).unwrap();
    let cancel = AtomicBool::new(false);
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    let entered = Cell::new(false);
    assert_eq!(
        session.with_handles(2, || session.with_handles(1, || {
            entered.set(true);
            Ok(())
        })),
        Err(Failure::BudgetExceeded)
    );
    assert!(!entered.get());
    let second = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    second.admit_result(4).unwrap();
    second.admit_result(4).unwrap();
    assert_eq!(second.admit_result(1), Err(Failure::BudgetExceeded));
    let third = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(
        third.with_handles(1, || {
            entered.set(true);
            Ok(())
        }),
        Err(Failure::Cancelled)
    );
    assert!(!entered.get());
}

#[test]
fn work_failure_is_latched_before_another_native_operation() {
    let limits = ProcessEvidenceLimits::default();
    let cancel = AtomicBool::new(false);
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    assert_eq!(
        session.with_handles::<()>(1, || Err(Failure::Unavailable)),
        Err(Failure::Unavailable)
    );
    let reached = Cell::new(false);
    let next = session.with_handles(1, || {
        reached.set(true);
        Ok(())
    });
    assert_eq!(
        next,
        Err(Failure::Unavailable),
        "work failure must remain the first failure"
    );
    assert!(
        !reached.get(),
        "failed session must not enter another native operation"
    );
    assert_eq!(session.check(), Err(Failure::Unavailable));
}

#[test]
fn unwinding_work_releases_the_exact_handle_reservation() {
    let limits = ProcessEvidenceLimits::new(1000, 16, 2, 8, 32, 0, 2).unwrap();
    let cancel = AtomicBool::new(false);
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &|| Ok(())).unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), Failure> =
            session.with_handles(2, || panic!("isolated native fixture unwind"));
    }));
    assert!(
        panic.is_err(),
        "fixture must reach the actual reserved work"
    );
    session.check().unwrap();
    let entered = Cell::new(false);
    let result = session.with_handles(2, || {
        entered.set(true);
        Ok(())
    });
    assert_eq!(result, Ok(()), "unwound reservation must be fully released");
    assert!(entered.get());
}

#[test]
fn final_authority_callback_cannot_return_success_after_the_original_deadline() {
    let limits = ProcessEvidenceLimits::new(100, 16, 2, 8, 32, 0, 2).unwrap();
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let deadline = started + Duration::from_millis(limits.max_duration_ms());
    let calls = Cell::new(0_u32);
    let check = || {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            // 真实最后一次同步回调越过同一个绝对期限，不创建新计时窗口。
            std::thread::sleep(
                deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(10),
            );
        }
        Ok(())
    };
    let session = ProcessNativeSession::new(&limits, started, &cancel, &check).unwrap();
    let work_ran = Cell::new(false);
    let result = session.with_handles(1, || {
        work_ran.set(true);
        Ok(())
    });
    assert!(work_ran.get());
    assert_eq!(
        calls.get(),
        3,
        "must reach the actual final authority callback"
    );
    assert!(Instant::now() >= deadline);
    assert_eq!(result, Err(Failure::Timeout));
    assert_eq!(session.check(), Err(Failure::Timeout));
    assert_eq!(calls.get(), 3, "latched timeout must not reenter authority");
}

#[test]
fn final_authority_callback_cancellation_is_observed_and_latched_before_success() {
    let limits = ProcessEvidenceLimits::default();
    let cancel = AtomicBool::new(false);
    let calls = Cell::new(0_u32);
    let check = || {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            cancel.store(true, Ordering::SeqCst);
        }
        Ok(())
    };
    let session = ProcessNativeSession::new(&limits, Instant::now(), &cancel, &check).unwrap();
    let work_ran = Cell::new(false);
    let result = session.with_handles(1, || {
        work_ran.set(true);
        Ok(())
    });
    assert!(work_ran.get());
    assert_eq!(calls.get(), 3);
    assert_eq!(result, Err(Failure::Cancelled));
    cancel.store(false, Ordering::SeqCst);
    assert_eq!(session.check(), Err(Failure::Cancelled));
    assert_eq!(calls.get(), 3);
}
