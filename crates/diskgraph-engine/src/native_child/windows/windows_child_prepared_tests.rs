//! 原 Job + 真实 pending Connect 的未出生责任；不以空 flag 代替原生 I/O。
use super::super::cleanup_progress::CleanupProgress;
use super::super::overlapped_pipe::OverlappedPipe;
use super::super::pipe_security::PipeSecurity;
use super::super::windows_birth_phase::WindowsBirthPhase;
use super::super::windows_cleanup_hooks::WindowsCleanupHooks;
use super::WindowsChild;
use crate::scan_worker_registry::ScanWorkerRegistry;
use crate::{EngineError, ScanWorkerRecovery};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn prepared_connect() -> WindowsChild {
    let mut owner = None;
    WindowsChild::prepare_job_into(&mut owner).unwrap();
    let child = owner.as_mut().unwrap();
    let name = super::pipe_name(&format!("diskgraph-prepared-job-{}", uuid::Uuid::new_v4()));
    let security = PipeSecurity::for_current_user().unwrap();
    OverlappedPipe::prepare_into(&name, &security, &mut child.stdout).unwrap();
    child.stdout.as_mut().unwrap().start_connect().unwrap();
    child.stdout.as_ref().unwrap().io_witness().unwrap();
    owner.take().unwrap()
}

#[test]
fn prepared_pending_connect_retains_capacity_until_real_job_and_io_complete() {
    let registry = ScanWorkerRegistry::new(1).unwrap();
    let recovery = ScanWorkerRecovery::new(Arc::clone(&registry));
    let reservation = registry.reserve().unwrap();
    let mut child = prepared_connect();
    let addresses = child.stdout.as_ref().unwrap().io_witness().unwrap();
    assert_eq!(
        child.poll_cleanup(Instant::now()).unwrap(),
        CleanupProgress::Pending
    );
    assert_eq!(
        child.stdout.as_ref().unwrap().io_witness().unwrap(),
        addresses
    );
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    assert!(!child.stdout_eof());
    reservation.retain(child);
    assert!(matches!(
        registry.reserve(),
        Err(EngineError::Business(
            diskgraph_core::BusinessError::ResourceExhausted
        ))
    ));
    assert!(!recovery.drain_until(Instant::now()).unwrap());
    assert_eq!(recovery.occupied_slots().unwrap(), 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !recovery.drain_until(deadline).unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(registry.reserve().is_ok());
}

#[test]
fn prepared_job_query_error_keeps_original_connect_owner_for_retry() {
    let mut child = prepared_connect();
    let addresses = child.stdout.as_ref().unwrap().io_witness().unwrap();
    WindowsCleanupHooks::arm(2);
    let result = child.poll_cleanup(Instant::now() + Duration::from_secs(5));
    WindowsCleanupHooks::disarm();
    assert!(result.is_err());
    assert_eq!(
        child.stdout.as_ref().unwrap().io_witness().unwrap(),
        addresses
    );
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.poll_cleanup(deadline).unwrap() != CleanupProgress::Complete {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(child.cleanup_state_for_test(), (false, false, true));
    assert!(
        !child.stdout_eof(),
        "connect disposal did not establish read EOF"
    );
}

#[test]
fn unknown_creation_phase_with_missing_leader_never_releases_original_job() {
    let mut child = prepared_connect();
    // 状态故障边界注入，不声称真的出现了未知内核创建结果；原 Job/Connect 均真实。
    child.birth_phase = WindowsBirthPhase::Creating;
    assert!(
        child
            .poll_cleanup(Instant::now() + Duration::from_secs(5))
            .is_err()
    );
    assert!(child.cleanup().is_err());
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    // 此 fixture 从未调用 CreateProcess，显式救援恢复已知未出生事实后实际清理。
    child.birth_phase = WindowsBirthPhase::Prepared;
    child.cleanup().unwrap();
}

#[test]
fn panic_after_prepared_connect_keeps_original_external_job_and_storage() {
    let mut owner = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        WindowsChild::prepare_job_into(&mut owner).unwrap();
        let child = owner.as_mut().unwrap();
        let name = super::pipe_name(&format!(
            "diskgraph-prepared-panic-{}",
            uuid::Uuid::new_v4()
        ));
        let security = PipeSecurity::for_current_user().unwrap();
        OverlappedPipe::prepare_into(&name, &security, &mut child.stdout).unwrap();
        child.stdout.as_mut().unwrap().start_connect().unwrap();
        child.stdout.as_ref().unwrap().io_witness().unwrap();
        std::panic::panic_any(String::from("original prepared birth panic"));
    }));
    assert_eq!(
        *result.unwrap_err().downcast::<String>().unwrap(),
        "original prepared birth panic"
    );
    let child = owner
        .as_mut()
        .expect("prepared panic lost original external owner");
    child.stdout.as_ref().unwrap().io_witness().unwrap();
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.poll_cleanup(deadline).unwrap() != CleanupProgress::Complete {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(!child.stdout_eof());
}

#[test]
fn checked_admission_rejects_before_job_without_advancing_lifecycle() {
    let mut owner = None;
    let mut phases = 0;
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    let result = WindowsChild::spawn_into_with_admission(
        &mut command,
        crate::native_child::ChildInputMode::Null,
        &mut owner,
        || Err("original admission cancellation"),
        || {
            phases += 1;
            Ok(())
        },
    );
    assert!(matches!(
        result,
        Err(crate::native_child::ChildSpawnError::Checkpoint {
            primary: "original admission cancellation",
            cleanup: None,
        })
    ));
    assert!(owner.is_none());
    assert_eq!(phases, 1);
}

#[test]
fn checked_admission_rejects_prepared_pipe_without_advancing_lifecycle() {
    let mut owner = None;
    let mut phases = 0;
    let mut admissions = 0;
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    let result = WindowsChild::spawn_into_with_admission(
        &mut command,
        crate::native_child::ChildInputMode::Null,
        &mut owner,
        || {
            admissions += 1;
            if admissions == 3 {
                Err("original prepared rejection")
            } else {
                Ok(())
            }
        },
        || {
            phases += 1;
            Ok(())
        },
    );
    assert!(matches!(
        result,
        Err(crate::native_child::ChildSpawnError::Checkpoint {
            primary: "original prepared rejection",
            cleanup: None,
        })
    ));
    assert_eq!(phases, 1);
    let child = owner
        .as_mut()
        .expect("prepared rejection lost original Job");
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    assert!(child.stdout.is_some());
    child.cleanup().unwrap();
}

#[test]
fn actual_pending_connect_wait_expires_on_original_admission_deadline() {
    let mut child = prepared_connect();
    let addresses = child.stdout.as_ref().unwrap().io_witness().unwrap();
    let deadline = Instant::now() + Duration::from_millis(20);
    let mut checks = 0;
    let result = super::windows_child_spawn::complete_prepared_connection(
        child.stdout.as_mut().unwrap(),
        &mut || {
            checks += 1;
            if Instant::now() >= deadline {
                Err(EngineError::Business(
                    diskgraph_core::BusinessError::BudgetExceeded,
                ))
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(
        result,
        Err(crate::native_child::ChildSpawnError::Checkpoint {
            primary: EngineError::Business(diskgraph_core::BusinessError::BudgetExceeded),
            cleanup: None,
        })
    ));
    assert!(checks >= 1);
    assert_eq!(
        child.stdout.as_ref().unwrap().io_witness().unwrap(),
        addresses
    );
    assert_eq!(child.cleanup_state_for_test(), (true, false, false));
    let cleanup_deadline = Instant::now() + Duration::from_secs(5);
    while child.poll_cleanup(cleanup_deadline).unwrap() != CleanupProgress::Complete {
        assert!(Instant::now() < cleanup_deadline);
        std::thread::yield_now();
    }
    assert!(!child.stdout_eof());
}
