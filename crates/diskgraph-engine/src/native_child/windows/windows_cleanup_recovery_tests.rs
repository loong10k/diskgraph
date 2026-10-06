//! 真实Windows进程+一次性失败注入；RED必须在Windows执行，不把交叉编译当验收。
use super::windows_child::WindowsChild;
use super::windows_cleanup_hooks::WindowsCleanupHooks as Hooks;
use super::windows_cleanup_rescue::WindowsCleanupRescue;
use super::windows_control_fixture::command;
use crate::native_child::{ChildError, ChildInputMode};
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{EngineError, ScanWorkerRecovery};
use diskgraph_core::BusinessError;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn spawn(directory: &std::path::Path) -> WindowsChild {
    let deadline = Instant::now() + Duration::from_secs(10);
    WindowsChild::spawn_with_input(
        &mut command("hold", directory),
        ChildInputMode::WorkerControl,
        || {
            if Instant::now() >= deadline {
                Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
            } else {
                Ok(())
            }
        },
    )
    .unwrap()
}

fn capture(child: &mut WindowsChild) -> WindowsCleanupRescue {
    match WindowsCleanupRescue::capture(child) {
        Ok(rescue) => rescue,
        Err(error) => {
            // 尚未启用注入；准备失败也先尝试原OS清理，再报告资格失败。
            let cleanup = child.cleanup();
            panic!("cannot capture independent rescue handles: {error:?}; cleanup={cleanup:?}");
        }
    }
}

fn original_failure(error: &ChildError) {
    assert_eq!(
        error
            .native_io_error()
            .and_then(std::io::Error::raw_os_error),
        Some(5)
    );
}

fn owner_case(stage: u8) {
    let directory = tempfile::tempdir().unwrap();
    let mut child = spawn(directory.path());
    let rescue = capture(&mut child);
    let observed = catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(rescue.active().unwrap(), 1);
        assert!(!rescue.waited().unwrap());
        Hooks::arm(stage);
        let first = child.cleanup();
        let state = child.cleanup_state_for_test();
        let after_failure = Hooks::counts();
        let second = child.cleanup();
        let after_retry = Hooks::counts();
        original_failure(first.as_ref().unwrap_err());
        assert_eq!(
            after_failure.2, 1,
            "must consume the intended actual cleanup boundary"
        );
        assert!(
            state.0 && !state.2,
            "failed cleanup lost original Job or claimed completion: {state:?}"
        );
        if stage == 1 {
            assert!(state.1, "failed wait must retain original process handle");
        }
        second.unwrap();
        if stage == 1 {
            assert!(
                after_retry.0 > after_failure.0,
                "retry must perform original native wait"
            );
        }
        assert!(
            after_retry.1 > after_failure.1,
            "retry must actually observe original Job accounting"
        );
        assert_eq!(rescue.active().unwrap(), 0);
        assert!(rescue.waited().unwrap());
    }));
    Hooks::disarm();
    let rescued = rescue.finish();
    if rescued.is_err() {
        std::mem::forget(rescue);
        std::mem::forget(child);
        let _ = directory.keep();
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("rescue failed; independent handles and original owner retained: {rescued:?}");
    }
    // 原实现可能已丢句柄，但独立救援已真实回收；允许恢复真实RED断言。
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn cleanup_wait_failure_retains_original_owner_until_actual_retry() {
    owner_case(1);
}

#[test]
fn cleanup_query_failure_retains_original_job_until_actual_retry() {
    owner_case(2);
}

#[test]
fn recovery_slot_survives_query_failure_until_original_job_is_observed_empty() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = spawn(directory.path());
    let rescue = capture(&mut child);
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let recovery = ScanWorkerRecovery::new(Arc::clone(&registry));
    let reservation = registry.reserve().unwrap();
    reservation.retain(child);
    drop(reservation);
    let observed = catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(rescue.active().unwrap(), 1);
        Hooks::arm(2);
        let first = recovery.drain();
        let occupied = recovery.occupied_slots().unwrap();
        let full = matches!(
            registry.reserve(),
            Err(EngineError::Business(BusinessError::ResourceExhausted))
        );
        let after_failure = Hooks::counts();
        let second = recovery.drain();
        let after_retry = Hooks::counts();
        assert!(
            matches!(first, Err(EngineError::Io(ref error)) if error.raw_os_error() == Some(5))
        );
        assert_eq!(after_failure.2, 1);
        assert_eq!(occupied, 1);
        assert!(full);
        assert!(second.unwrap());
        assert!(
            after_retry.1 > after_failure.1,
            "registry must not release slot after fake idempotent success"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
        assert_eq!(rescue.active().unwrap(), 0);
        assert!(rescue.waited().unwrap());
    }));
    Hooks::disarm();
    let rescued = rescue.finish();
    let drained = recovery.drain();
    if rescued.is_err() || !matches!(drained, Ok(true)) {
        std::mem::forget(rescue);
        std::mem::forget(recovery);
        let _ = directory.keep();
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!(
            "recovery rescue incomplete; owners retained: rescue={rescued:?}, drain={drained:?}"
        );
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}
