//! Linux 原 pidfd 正常回收的真实运行契约，不调用旧 Unix/macOS 数值组许可。
use super::ChildSpawnError;
use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_launcher_fixture::AtomicLauncherFixture;
use super::linux_atomic_launcher_test_support::{
    assert_reaped, check, complete, deadline, duplicate,
};
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

fn drain_eofs(child: &mut LinuxAtomicChild, end: Instant) -> Vec<u8> {
    let mut output = Vec::new();
    while !child.stdout_eof() || !child.stderr_eof() {
        check(end).unwrap();
        if let Some(bytes) = child.read_stdout().unwrap() {
            assert!(output.len() + bytes.len() <= 4096);
            output.extend_from_slice(bytes);
        }
        if let Some(bytes) = child.read_stderr().unwrap() {
            assert!(bytes.len() <= 4096);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    output
}

fn assert_wait_retained(pidfd: &OwnedFd) {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            libc::waitid(
                libc::P_PIDFD,
                pidfd.as_raw_fd() as u32,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        },
        0,
        "original pidfd wait was consumed before authorization"
    );
    assert_ne!(unsafe { info.si_pid() }, 0);
    assert_eq!(info.si_code, libc::CLD_EXITED);
}

#[test]
fn two_real_output_eofs_do_not_permit_waiting_a_live_original_process() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut outside = None;
    let mut child = fixture
        .launcher("closed_live")
        .spawn(end, &mut || check(end), &mut outside)
        .unwrap();
    let original = duplicate(&child);
    assert!(
        String::from_utf8(drain_eofs(&mut child, end))
            .unwrap()
            .contains("CLOSED_LIVE_READY")
    );
    assert!(!child.poll().unwrap());
    assert!(!child.poll_normal_exit(&mut || check(end)).unwrap());
    assert!(!child.reaped());
    child.request_control_close().unwrap();
    complete(&mut child, end);
    assert_eq!(child.exit_code(), Some(0));
    assert_reaped(&original);
}

#[test]
fn first_checkpoint_preserves_nonclone_primary_and_live_original_process() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut outside = None;
    let mut child = fixture
        .launcher("closed_live")
        .spawn(end, &mut || check(end), &mut outside)
        .unwrap();
    let original = duplicate(&child);
    drain_eofs(&mut child, end);
    let error = child
        .poll_normal_exit(&mut || {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "original-authority",
            ))
        })
        .unwrap_err();
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None }
        if primary.kind() == io::ErrorKind::PermissionDenied && primary.to_string() == "original-authority")
    );
    assert!(!child.poll().unwrap());
    assert!(!child.reaped());
    child.request_control_close().unwrap();
    complete(&mut child, end);
    assert_reaped(&original);
}

#[test]
fn final_pre_wait_checkpoint_failure_retains_original_wait_and_nonclone_primary() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut outside = None;
    let mut child = fixture
        .launcher("image")
        .spawn(end, &mut || check(end), &mut outside)
        .unwrap();
    let original = duplicate(&child);
    child.request_control_close().unwrap();
    drain_eofs(&mut child, end);
    while !child.poll().unwrap() {
        check(end).unwrap();
        std::thread::yield_now();
    }
    assert_wait_retained(&original);
    let mut calls = 0;
    let error = child
        .poll_normal_exit(&mut || {
            calls += 1;
            if calls == 2 {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "original-pre-wait",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(calls, 2);
    assert!(
        matches!(error, ChildSpawnError::Checkpoint { primary, cleanup: None }
        if primary.kind() == io::ErrorKind::Interrupted && primary.to_string() == "original-pre-wait")
    );
    assert!(
        !child.reaped(),
        "final authorization must precede original pidfd wait"
    );
    assert_wait_retained(&original);
    complete(&mut child, end);
    assert_eq!(child.exit_code(), Some(0));
    assert_reaped(&original);
}

#[test]
fn independent_pidfds_keep_live_permission_separate_from_nonzero_completion() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut outside = None;
    let mut live = fixture
        .launcher("closed_live")
        .spawn(end, &mut || check(end), &mut outside)
        .unwrap();
    let live_original = duplicate(&live);
    drain_eofs(&mut live, end);
    let mut done = fixture
        .launcher("nonzero")
        .spawn(end, &mut || check(end), &mut outside)
        .unwrap();
    let done_original = duplicate(&done);
    assert_ne!(live.pid_for_test(), done.pid_for_test());
    done.request_control_close().unwrap();
    complete(&mut done, end);
    assert_eq!(done.exit_code(), Some(7));
    assert_reaped(&done_original);
    assert!(!live.poll().unwrap());
    assert!(!live.poll_normal_exit(&mut || check(end)).unwrap());
    assert!(!live.reaped());
    live.request_control_close().unwrap();
    complete(&mut live, end);
    assert_eq!(live.exit_code(), Some(0));
    assert_reaped(&live_original);
}
