//! 真实Windows出生后原错误/panic必须保留catch外唯一owner；救援不计成功证据。
use super::windows_birth_test_hook::WindowsBirthTestHook;
use super::windows_child::WindowsChild;
use super::windows_cleanup_hooks::WindowsCleanupHooks as Hooks;
use super::windows_cleanup_rescue::WindowsCleanupRescue;
use super::windows_control_fixture::command;
use crate::native_child::{ChildInputMode, ChildSpawnError};
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_owned_failure::ScanWorkerOwnedFailure;
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{EngineError, ScanWorkerRecovery};
use diskgraph_core::BusinessError;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn birth_case(panic_after_birth: bool, cleanup_stage: u8) {
    let directory = tempfile::tempdir().unwrap();
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let recovery = ScanWorkerRecovery::new(Arc::clone(&registry));
    let reservation = registry.reserve().unwrap();
    let mut owner = None;
    let rescue = Rc::new(RefCell::new(None::<WindowsCleanupRescue>));
    let observer_rescue = Rc::clone(&rescue);
    let hook = WindowsBirthTestHook::install(move |child| {
        *observer_rescue.borrow_mut() = Some(WindowsCleanupRescue::capture(child).unwrap());
    });
    let expires = Instant::now() + Duration::from_secs(10);
    // 唯一owner、槽位和独立救援都位于出生及所有断言的catch边界之外。
    let observed = catch_unwind(AssertUnwindSafe(|| {
        let birth = catch_unwind(AssertUnwindSafe(|| {
            WindowsChild::spawn_into(
                &mut command("hold", directory.path()),
                ChildInputMode::WorkerControl,
                &mut owner,
                || {
                    if rescue.borrow().is_some() {
                        if panic_after_birth {
                            std::panic::panic_any(String::from("original Windows birth panic"));
                        }
                        return Err(EngineError::Poisoned);
                    }
                    if Instant::now() >= expires {
                        return Err(EngineError::Business(BusinessError::BudgetExceeded));
                    }
                    Ok(())
                },
            )
        }));
        assert!(
            rescue.borrow().is_some(),
            "must witness actual native birth"
        );
        if panic_after_birth {
            let payload = birth.expect_err("original checkpoint must panic");
            assert_eq!(
                *payload.downcast::<String>().unwrap(),
                "original Windows birth panic"
            );
        } else {
            assert!(matches!(
                birth.unwrap(),
                Err(ChildSpawnError::Checkpoint {
                    primary: EngineError::Poisoned,
                    ..
                })
            ));
        }
        assert!(
            owner.is_some(),
            "postbirth failure lost original external owner"
        );
        Hooks::arm(cleanup_stage);
        let (failure, retained) = ScanWorkerOwnedFailure::dispose(
            ScanWorkerFailure::Checkpoint {
                primary: EngineError::Poisoned,
                cleanup: None,
            },
            owner.take().unwrap(),
        )
        .into_parts();
        // 先转移实际保留owner，再检查错误；断言panic也不遗失责任。
        if let Some(child) = retained {
            reservation.retain(child);
        }
        let after_failure = Hooks::counts();
        assert!(matches!(failure, ScanWorkerFailure::Checkpoint {
            primary: EngineError::Poisoned, cleanup: Some(ref error)
        } if error.native_io_error().and_then(std::io::Error::raw_os_error) == Some(5)));
        assert_eq!(after_failure.2, 1);
        assert_eq!(recovery.occupied_slots().unwrap(), 1);
        assert!(matches!(
            registry.reserve(),
            Err(EngineError::Business(BusinessError::ResourceExhausted))
        ));
        assert!(recovery.drain().unwrap());
        let after_retry = Hooks::counts();
        assert!(
            after_retry.0 > after_failure.0,
            "retry needs actual original leader wait"
        );
        assert!(
            after_retry.1 > after_failure.1,
            "retry needs actual original Job accounting"
        );
        assert_eq!(recovery.occupied_slots().unwrap(), 0);
        assert!(rescue.borrow().as_ref().unwrap().waited().unwrap());
        assert_eq!(rescue.borrow().as_ref().unwrap().active().unwrap(), 0);
    }));
    drop(hook);
    Hooks::disarm();
    // finally独立救援只保证失败测试不遗留真实子进程，不能使上述原owner断言通过。
    let rescued = rescue
        .borrow()
        .as_ref()
        .map_or(Ok(()), WindowsCleanupRescue::finish);
    if let Some(child) = owner.take() {
        reservation.retain(child);
    }
    drop(reservation);
    let drained = recovery.drain();
    if rescued.is_err() || !matches!(drained, Ok(true)) {
        std::mem::forget(rescue);
        std::mem::forget(recovery);
        let _ = directory.keep();
        if let Err(payload) = observed {
            resume_unwind(payload);
        }
        panic!("birth rescue incomplete; original and guardian responsibility retained");
    }
    if let Err(payload) = observed {
        resume_unwind(payload);
    }
}

#[test]
fn postbirth_panic_keeps_external_owner_through_failed_wait_and_recovery() {
    birth_case(true, 1);
}

#[test]
fn postbirth_checkpoint_error_keeps_external_owner_through_failed_wait() {
    birth_case(false, 1);
}

#[test]
fn postbirth_checkpoint_error_keeps_external_owner_through_failed_job_query() {
    birth_case(false, 2);
}

#[test]
fn actual_create_process_failure_retains_external_job_until_observed_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.current_dir(temp.path().join("missing-working-directory"));
    let mut owner = None;
    let result = WindowsChild::spawn_into(&mut command, ChildInputMode::Null, &mut owner, || {
        Ok::<(), ()>(())
    });
    match result {
        Err(ChildSpawnError::Operation(crate::native_child::ChildError::NativeIo {
            context,
            source,
        })) => {
            assert_eq!(context, "CreateProcessW(JOB_LIST,HANDLE_LIST)");
            let code = source
                .raw_os_error()
                .expect("actual native CreateProcess error code");
            println!("DG_ACTUAL_CREATE_PROCESS_FAILURE={code}");
        }
        _ => panic!("must reach the actual native CreateProcess failure"),
    }
    let child = owner
        .as_mut()
        .expect("actual failed CreateProcess must retain original Job");
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    child.cleanup().unwrap();
    assert_eq!(child.cleanup_state_for_test(), (false, false, true));
}
