use super::LinuxSupervisorLauncher;
use crate::recovery_slot::SlotReservation;
use std::fs::File;
use std::time::{Duration, Instant};

// 只检验出生前模式准入，不将手设保护字段当成真实客户身份、隐私或exec证明。
fn reject_before_birth(protected: bool, lifecycle: bool, executor: bool) {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("original-slot");
    let until = Instant::now() + Duration::from_secs(5);
    let reservation = SlotReservation::acquire(
        File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap(),
        until,
    )
    .unwrap();
    let mut prepared = LinuxSupervisorLauncher::prepare(
        File::open(std::env::current_exe().unwrap()).unwrap(),
        vec![c"admission-only".to_owned()],
        Vec::new(),
        (23111, 23111),
    )
    .unwrap();
    prepared.executor_protection = protected;
    let result = if executor {
        prepared.spawn_executor_lifecycle(reservation, [7; 32], until)
    } else if lifecycle {
        prepared.spawn_lifecycle(reservation, [7; 32], until)
    } else {
        prepared.spawn(reservation, [7; 32], until)
    };
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("invalid executor mode reached child birth"),
    };
    let (error, owner) = error.into_parts();
    assert!(owner.is_none(), "mode refusal must precede child birth");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read(&path).unwrap(), b"DGSL01C\n");
    SlotReservation::acquire(
        File::options().read(true).write(true).open(&path).unwrap(),
        until,
    )
    .unwrap()
    .abort_before_birth(until)
    .unwrap();
}

#[test]
fn unprotected_launcher_cannot_start_executor() {
    reject_before_birth(false, true, true);
}

#[test]
fn protected_executor_cannot_receive_legacy_supervisor_materials() {
    reject_before_birth(true, false, false);
    reject_before_birth(true, true, false);
}

#[test]
fn executor_without_held_worker_is_refused_and_unborn_slot_is_reusable() {
    reject_before_birth(true, true, true);
}
