//! 真实生产probe执行链的恢复RED；原budget取消/panic及真实cleanup失败不能丢宿主责任。
use super::windows_birth_test_hook::WindowsBirthTestHook;
use super::windows_cleanup_hooks::WindowsCleanupHooks as Hooks;
use super::windows_cleanup_rescue::WindowsCleanupRescue;
use super::windows_control_fixture::command;
use crate::ProbeHost;
use crate::live_evidence::{ProbeLimits, run_managed_probe_for_test};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

fn managed_probe_case(panic_after_birth: bool, stage: u8) {
    let directory = tempfile::tempdir().unwrap();
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let limits = ProbeLimits {
        timeout: Duration::from_secs(10),
        ..ProbeLimits::default()
    };
    let cancel = Arc::clone(&limits.cancel);
    let rescue = Rc::new(RefCell::new(None::<WindowsCleanupRescue>));
    let hook_rescue = Rc::clone(&rescue);
    let hook = WindowsBirthTestHook::install(move |child| {
        *hook_rescue.borrow_mut() = Some(WindowsCleanupRescue::capture(child).unwrap());
        Hooks::arm(stage);
        if panic_after_birth {
            std::panic::panic_any(String::from("original managed Windows probe panic"));
        }
        cancel.store(true, Ordering::Release);
    });
    let observed = catch_unwind(AssertUnwindSafe(|| {
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_managed_probe_for_test(&mut command("hold", directory.path()), &limits, &host)
        }));
        assert!(
            rescue.borrow().is_some(),
            "actual probe birth must be witnessed"
        );
        if panic_after_birth {
            let payload = result.expect_err("original probe callback must unwind");
            assert_eq!(
                *payload.downcast::<String>().unwrap(),
                "original managed Windows probe panic"
            );
        } else {
            let error = result.unwrap().unwrap_err();
            assert!(
                error.contains("probe cancelled"),
                "original cancellation lost: {error}"
            );
            assert!(
                error.contains("cleanup"),
                "original cleanup diagnostic lost: {error}"
            );
        }
        let after_failure = Hooks::counts();
        assert_eq!(
            after_failure.2, 1,
            "actual cleanup boundary must consume the single fault"
        );
        assert_eq!(
            recovery.occupied_slots().unwrap(),
            1,
            "failed production probe lost its original owner instead of retaining capacity"
        );
        assert!(
            host.registry.reserve().is_err(),
            "unreaped probe must block new births"
        );
        assert!(recovery.drain().unwrap());
        let after_retry = Hooks::counts();
        assert!(
            after_retry.0 > after_failure.0,
            "recovery needs original native wait"
        );
        assert!(
            after_retry.1 > after_failure.1,
            "recovery needs original Job accounting"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
        assert!(rescue.borrow().as_ref().unwrap().waited().unwrap());
        assert_eq!(rescue.borrow().as_ref().unwrap().active().unwrap(), 0);
    }));
    drop(hook);
    Hooks::disarm();
    let rescued = rescue
        .borrow()
        .as_ref()
        .map_or(Ok(()), WindowsCleanupRescue::finish);
    let drained = recovery.drain();
    if rescued.is_err() || !matches!(drained, Ok(true)) {
        std::mem::forget(rescue);
        std::mem::forget(host);
        std::mem::forget(recovery);
        let _ = directory.keep();
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("probe rescue incomplete; independent and original responsibilities retained");
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn managed_probe_cancel_wait_failure_retains_original_owner_and_capacity() {
    managed_probe_case(false, 1);
}

#[test]
fn managed_probe_cancel_query_failure_retains_original_owner_and_capacity() {
    managed_probe_case(false, 2);
}

#[test]
fn managed_probe_panic_wait_failure_retains_original_owner_and_capacity() {
    managed_probe_case(true, 1);
}

#[test]
fn expired_managed_probe_transfers_original_owner_without_legacy_wait() {
    let directory = tempfile::tempdir().unwrap();
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let limits = ProbeLimits {
        timeout: Duration::from_secs(10),
        ..ProbeLimits::default()
    };
    let rescue = Rc::new(RefCell::new(None::<WindowsCleanupRescue>));
    let hook_rescue = Rc::clone(&rescue);
    let hook = WindowsBirthTestHook::install(move |child| {
        *hook_rescue.borrow_mut() = Some(WindowsCleanupRescue::capture(child).unwrap());
        Hooks::arm(1);
        // 真实出生后耗尽整次原预算；没有新期限、虚构退出或伪 Job0。
        std::thread::sleep(Duration::from_secs(11));
    });
    let observed = catch_unwind(AssertUnwindSafe(|| {
        let result =
            run_managed_probe_for_test(&mut command("hold", directory.path()), &limits, &host);
        assert!(
            rescue.borrow().is_some(),
            "actual expired probe birth must be witnessed"
        );
        let error = result.unwrap_err();
        assert!(
            error.contains("probe deadline exceeded"),
            "original deadline lost: {error}"
        );
        println!("DG_EXPIRED_PROBE_BOUNDARY={}", Hooks::counts().2);
        assert_eq!(
            Hooks::counts().2,
            0,
            "expired product probe must not enter legacy wait"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        assert!(
            host.registry.reserve().is_err(),
            "pending original Job must retain capacity"
        );
        Hooks::disarm();
        assert!(recovery.drain().unwrap());
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
        assert!(rescue.borrow().as_ref().unwrap().waited().unwrap());
        assert_eq!(rescue.borrow().as_ref().unwrap().active().unwrap(), 0);
    }));
    drop(hook);
    Hooks::disarm();
    let rescued = rescue
        .borrow()
        .as_ref()
        .map_or(Ok(()), WindowsCleanupRescue::finish);
    let drained = recovery.drain();
    if rescued.is_err() || !matches!(drained, Ok(true)) {
        std::mem::forget(rescue);
        std::mem::forget(host);
        std::mem::forget(recovery);
        let _ = directory.keep();
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("expired probe rescue incomplete; original responsibilities retained");
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}
