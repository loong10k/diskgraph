//! 真实Git私有目录与真实child的共同保留RED；目录准入失败不算目标行为失败。
use super::windows_birth_test_hook::WindowsBirthTestHook;
use super::windows_cleanup_hooks::WindowsCleanupHooks as Hooks;
use super::windows_cleanup_rescue::WindowsCleanupRescue;
use super::windows_control_fixture::command;
use crate::ProbeHost;
use crate::live_evidence::{
    ProbeDirectoryWitness, ProbeLimits, qualify_private_probe_directory_for_test,
    run_managed_private_probe_for_test,
};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

#[test]
fn real_git_private_directory_creation_and_completion_qualifies_resource_fixture() {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    qualify_private_probe_directory_for_test(&ProbeLimits::default(), &host)
        .expect("native GitPrivateDirectory creation/write/identity/complete prerequisite failed");
    assert!(recovery.drain().unwrap());
    println!("DG_PROBE_DIRECTORY prerequisite_real_creation_and_complete=true");
}

fn resource_case(panic_after_birth: bool) {
    let (host, recovery) = ProbeHost::new(1).unwrap();
    let limits = ProbeLimits {
        timeout: Duration::from_secs(10),
        ..ProbeLimits::default()
    };
    let cancel = Arc::clone(&limits.cancel);
    let directory = Rc::new(RefCell::new(None::<ProbeDirectoryWitness>));
    let rescue = Rc::new(RefCell::new(None::<WindowsCleanupRescue>));
    let hook_rescue = Rc::clone(&rescue);
    let hook = WindowsBirthTestHook::install(move |child| {
        *hook_rescue.borrow_mut() = Some(WindowsCleanupRescue::capture(child).unwrap());
        Hooks::arm(1);
        if panic_after_birth {
            std::panic::panic_any(String::from("original private Git probe panic"));
        }
        cancel.store(true, Ordering::Release);
    });
    let observed = catch_unwind(AssertUnwindSafe(|| {
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_managed_private_probe_for_test(
                &mut command("hold", Path::new(".")),
                &limits,
                &host,
                |witness| {
                    *directory.borrow_mut() = Some(witness);
                    println!("DG_PROBE_DIRECTORY actual_private_identity_captured=true");
                },
            )
        }));
        assert!(
            directory.borrow().is_some(),
            "prerequisite GitPrivateDirectory failed before birth: {result:?}"
        );
        assert!(
            rescue.borrow().is_some(),
            "real child birth prerequisite absent: {result:?}"
        );
        if panic_after_birth {
            let payload =
                result.expect_err("original callback panic must survive Git directory Drop");
            assert_eq!(
                *payload.downcast::<String>().unwrap(),
                "original private Git probe panic"
            );
        } else {
            let error = result.unwrap().unwrap_err();
            assert!(
                error.contains("probe cancelled"),
                "original cancellation lost: {error}"
            );
            assert!(
                error.contains("cleanup"),
                "actual cleanup failure lost: {error}"
            );
        }
        let before_retry = Hooks::counts();
        assert_eq!(before_retry.2, 1);
        assert!(
            directory
                .borrow()
                .as_ref()
                .unwrap()
                .same_directory_exists()
                .unwrap(),
            "complete/Drop removed the original private Git directory before explicit owner recovery"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        assert!(
            host.registry.reserve().is_err(),
            "retained directory/child must keep capacity occupied"
        );
        assert!(recovery.drain().unwrap());
        let after_retry = Hooks::counts();
        assert!(
            after_retry.0 > before_retry.0,
            "same original leader requires actual native wait"
        );
        assert!(
            after_retry.1 > before_retry.1,
            "same original Job requires actual zero accounting"
        );
        assert!(rescue.borrow().as_ref().unwrap().waited().unwrap());
        assert_eq!(rescue.borrow().as_ref().unwrap().active().unwrap(), 0);
        assert!(
            directory.borrow().as_ref().unwrap().absent().unwrap(),
            "recovery must finish directory cleanup after child reaping"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
    }));
    drop(hook);
    Hooks::disarm();
    let rescued = rescue
        .borrow()
        .as_ref()
        .map_or(Ok(()), WindowsCleanupRescue::finish);
    let drained = recovery.drain();
    let directory_rescued = if rescued.is_ok() && matches!(drained, Ok(true)) {
        directory
            .borrow()
            .as_ref()
            .map_or(Ok(()), ProbeDirectoryWitness::rescue_after_child_reaped)
    } else {
        Err("original child responsibility incomplete; directory retained".into())
    };
    if rescued.is_err() || !matches!(drained, Ok(true)) || directory_rescued.is_err() {
        std::mem::forget(rescue);
        std::mem::forget(directory);
        std::mem::forget(host);
        std::mem::forget(recovery);
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!(
            "private probe rescue incomplete; original owners and directory responsibility retained"
        );
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn private_git_complete_retains_original_directory_after_cancel_cleanup_failure() {
    resource_case(false);
}

#[test]
fn private_git_drop_retains_original_directory_after_panic_cleanup_failure() {
    resource_case(true);
}
