//! Linux 真实 pidfd、期限和线程 waitid 拒绝的恢复合同；来源：PF-06。
use super::ScanWorkerRegistry;
use crate::EngineError;
use crate::scan_worker_owner_slot::ScanWorkerOwnerSlot;
use crate::scan_worker_recovery_fixture::ScanWorkerRecoveryFixture;
use crate::scan_worker_recovery_tests::deny_waitid_for_this_thread;
use diskgraph_core::BusinessError;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn retained_state(registry: &ScanWorkerRegistry) -> (bool, bool) {
    let slots = registry.slots.lock().unwrap();
    match &slots[0] {
        ScanWorkerOwnerSlot::Retained(child) => {
            (child.physically_exited().unwrap(), child.reaped())
        }
        _ => panic!("original owner must remain in original retained slot"),
    }
}

fn finish(fixture: &ScanWorkerRecoveryFixture) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = fixture.recovery.drain_until(deadline);
        if matches!(result, Ok(true)) {
            return;
        }
        if result.is_err() || Instant::now() >= deadline {
            // 断言前先由未过滤宿主处置原 owner，不能因测试失败丢弃活进程。
            let cleanup = fixture.recovery.drain();
            panic!("bounded original recovery failed: {result:?}; cleanup={cleanup:?}");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn expired_deadline_keeps_live_original_child_and_capacity_on_every_retry() {
    let fixture = ScanWorkerRecoveryFixture::new();
    let reservation = fixture.registry.reserve().unwrap();
    let child = fixture.launch_original();
    assert!(!child.physically_exited().unwrap());
    reservation.retain(child);
    drop(reservation);
    let expired = Instant::now() - Duration::from_millis(1);
    let observations: Vec<_> = (0..3)
        .map(|_| {
            let result = fixture.recovery.drain_until(expired);
            let state = retained_state(&fixture.registry);
            let denied = matches!(
                fixture.registry.reserve(),
                Err(EngineError::Business(BusinessError::ResourceExhausted))
            );
            (result, state, denied)
        })
        .collect();
    finish(&fixture);
    for (result, state, denied) in observations {
        assert!(!result.unwrap());
        assert_eq!(state, (false, false));
        assert!(denied);
    }
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
}

#[test]
fn actual_wait_denial_keeps_original_owner_until_unfiltered_deadline_recovery() {
    let fixture = ScanWorkerRecoveryFixture::new();
    let reservation = fixture.registry.reserve().unwrap();
    reservation.retain(fixture.launch_original());
    drop(reservation);
    let registry = Arc::clone(&fixture.registry);
    let errors = std::thread::spawn(move || {
        deny_waitid_for_this_thread();
        (0..3)
            .map(|_| registry.drain_until(Instant::now() + Duration::from_secs(5)))
            .collect::<Vec<_>>()
    })
    .join()
    .unwrap();
    let state = retained_state(&fixture.registry);
    let denied = matches!(
        fixture.registry.reserve(),
        Err(EngineError::Business(BusinessError::ResourceExhausted))
    );
    finish(&fixture);
    for result in errors {
        let error = result.unwrap_err();
        assert!(
            matches!(error.primary(), EngineError::Io(source)
            if source.raw_os_error() == Some(libc::EACCES)),
            "{error:?}"
        );
    }
    assert!(
        !state.1,
        "physical exit cannot consume original wait under denial"
    );
    assert!(denied);
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
}

#[test]
fn deadline_cleanup_releases_capacity_only_after_original_pidfd_wait_consumption() {
    let fixture = ScanWorkerRecoveryFixture::new();
    let reservation = fixture.registry.reserve().unwrap();
    let child = fixture.launch_original();
    let raw = unsafe { libc::fcntl(child.pidfd_for_test(), libc::F_DUPFD_CLOEXEC, 3) };
    assert!(raw >= 0);
    let original = unsafe { OwnedFd::from_raw_fd(raw) };
    reservation.retain(child);
    drop(reservation);
    finish(&fixture);
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let waited = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            original.as_raw_fd() as u32,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        )
    };
    assert_eq!(waited, -1);
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    let mut view = libc::pollfd {
        fd: original.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    assert_eq!(unsafe { libc::poll(&mut view, 1, 0) }, 1);
    assert_ne!(view.revents & libc::POLLIN, 0);
    assert_eq!(fixture.recovery.occupied_slots().unwrap(), 0);
    let renewed = fixture.registry.reserve().unwrap();
    drop(renewed);
    eprintln!("DG_LINUX_DEADLINE_ORIGINAL_WAIT_CONSUMED=1");
}
