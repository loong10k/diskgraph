//! 原子出生后首 Ready 前的真实错误与回收，不通过数字 PID 重新寻找 owner。

use super::linux_atomic_launch_failure::LinuxAtomicLaunchFailure;
use super::linux_atomic_launcher_fixture::AtomicLauncherFixture;
use super::linux_atomic_launcher_test_support::{
    assert_reaped, check, complete, deadline, duplicate, isolated, line,
};
use super::{ChildError, ChildSpawnError};
use std::cell::{Cell, RefCell};
use std::io;
use std::os::fd::AsRawFd;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn primary<E>(
    result: Result<super::linux_atomic_child::LinuxAtomicChild, LinuxAtomicLaunchFailure<E>>,
) -> Result<super::linux_atomic_child::LinuxAtomicChild, ChildSpawnError<E>> {
    result.map_err(|failure| {
        let (error, owner) = failure.into_parts();
        assert!(
            owner.is_none(),
            "normal failure cleanup consumed original wait"
        );
        error
    })
}

fn original_errno<E>(
    result: Result<super::linux_atomic_child::LinuxAtomicChild, LinuxAtomicLaunchFailure<E>>,
) -> i32 {
    match primary(result) {
        Err(ChildSpawnError::Operation(error)) => {
            error.native_io_error().unwrap().raw_os_error().unwrap()
        }
        Err(ChildSpawnError::Checkpoint { .. }) => panic!("unexpected checkpoint classification"),
        Ok(mut child) => {
            child.cleanup().unwrap();
            panic!("expected real native failure");
        }
    }
}

#[test]
fn fatal_before_first_init_is_reaped_through_original_kernel_pidfd() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let original = RefCell::new(None);
    let seen = Cell::new(false);
    let result = fixture.launcher("image").spawn_observed(
        end,
        &mut || check(end),
        &mut unwind_owner,
        &mut |child| {
            *original.borrow_mut() = Some(duplicate(child));
            seen.set(true);
            assert_eq!(
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        child.pidfd_for_test(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                },
                0
            );
            Ok(())
        },
    );
    assert!(seen.get(), "real pidfd owner exists before any init permit");
    assert!(matches!(
        primary(result),
        Err(ChildSpawnError::Operation(_))
    ));
    assert_reaped(original.borrow().as_ref().unwrap());
}

#[test]
fn startup_channel_eof_causes_actual_natural_exit_and_original_reap() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let original = RefCell::new(None);
    let actual_exit = Cell::new(None);
    let result = fixture.launcher("image").spawn_observed(
        end,
        &mut || check(end),
        &mut unwind_owner,
        &mut |child| {
            *original.borrow_mut() = Some(duplicate(child));
            child.close_startup_for_test();
            while !child.poll()? {
                check(end)?;
                std::thread::yield_now();
            }
            actual_exit.set(child.exit_code());
            Ok(())
        },
    );
    assert_eq!(
        actual_exit.get(),
        Some(127),
        "child actually consumes EOF before Ready"
    );
    assert!(matches!(
        primary(result),
        Err(ChildSpawnError::Operation(_))
    ));
    assert_reaped(original.borrow().as_ref().unwrap());
}

#[test]
fn after_birth_checkpoint_keeps_nonclone_primary_and_performs_actual_wait() {
    struct OriginalDenied(&'static str);
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let original = RefCell::new(None);
    let seen = Cell::new(false);
    let result = fixture.launcher("echo").spawn_observed(
        end,
        &mut || {
            if seen.get() {
                Err(OriginalDenied("original-request-denied"))
            } else {
                Ok(())
            }
        },
        &mut unwind_owner,
        &mut |child| {
            *original.borrow_mut() = Some(duplicate(child));
            seen.set(true);
            Ok(())
        },
    );
    match primary(result) {
        Err(ChildSpawnError::Checkpoint { primary, cleanup }) => {
            assert_eq!(primary.0, "original-request-denied");
            assert!(cleanup.is_none(), "actual reap succeeded");
        }
        Err(ChildSpawnError::Operation(error)) => panic!("lost original denial {error:?}"),
        Ok(mut child) => {
            child.cleanup().unwrap();
            panic!("denial was ignored");
        }
    }
    assert_reaped(original.borrow().as_ref().unwrap());
}

#[test]
fn parent_observer_panic_reaps_before_original_payload_resumes() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let original = RefCell::new(None);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _ = fixture.launcher("echo").spawn_observed(
            end,
            &mut || check(end),
            &mut unwind_owner,
            &mut |child| {
                *original.borrow_mut() = Some(duplicate(child));
                std::panic::panic_any("atomic-launch-observer-sentinel");
            },
        );
    }));
    let payload = result.expect_err("specified parent panic");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"atomic-launch-observer-sentinel")
    );
    assert_reaped(original.borrow().as_ref().unwrap());
}

#[test]
fn original_pidfd_external_reap_does_not_target_other_live_child() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let mut other = fixture
        .launcher("echo")
        .spawn(end, &mut || check(end), &mut unwind_owner)
        .unwrap();
    let other_original = duplicate(&other);
    let original = RefCell::new(None);
    let result = fixture.launcher("image").spawn_observed(
        end,
        &mut || check(end),
        &mut unwind_owner,
        &mut |child| {
            let pidfd = duplicate(child);
            assert_eq!(
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        pidfd.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                },
                0
            );
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            loop {
                if unsafe {
                    libc::waitid(
                        libc::P_PIDFD,
                        pidfd.as_raw_fd() as u32,
                        &mut info,
                        libc::WEXITED,
                    )
                } == 0
                {
                    break;
                }
                assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EINTR));
            }
            *original.borrow_mut() = Some(pidfd);
            Ok(())
        },
    );
    let (failure, owner) = match result {
        Err(failure) => failure.into_parts(),
        Ok(mut child) => {
            child.cleanup().unwrap();
            panic!("external reap is not a permit");
        }
    };
    let owner =
        owner.expect("unconsumed original wait obligation is returned even after external reap");
    assert!(owner.physically_exited().unwrap());
    assert!(!owner.reaped());
    let error = match failure {
        ChildSpawnError::Operation(error) => error,
        ChildSpawnError::Checkpoint { .. } => panic!("lost native failure"),
    };
    // 原生 ECHILD 可在主错或 cleanup 链；绝不解析 errno 文本。
    fn has_echild(error: &ChildError) -> bool {
        match error {
            ChildError::NativeIo { source, .. } => source.raw_os_error() == Some(libc::ECHILD),
            ChildError::Cleanup { primary, cleanup } => has_echild(primary) || has_echild(cleanup),
            _ => false,
        }
    }
    assert!(
        has_echild(&error),
        "original wait capability was consumed {error:?}"
    );
    assert_reaped(original.borrow().as_ref().unwrap());
    assert!(
        !other.poll().unwrap(),
        "never retarget a different live process"
    );
    other.request_control_close().unwrap();
    assert_eq!(line(&complete(&mut other, end).0, "STDIN_EOF="), "true");
    assert_reaped(&other_original);
    println!("DG_ATOMIC_PRODUCTION_EXTERNAL_REAP pid_reuse_verified=false");
}

pub(super) fn deny_syscall(number: libc::c_long) {
    let instructions = [
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: number as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: 0x0005_0000 | libc::EACCES as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: 0x7fff_0000,
        },
    ];
    let program = libc::sock_fprog {
        len: instructions.len() as u16,
        filter: instructions.as_ptr().cast_mut(),
    };
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) },
        0
    );
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_SECCOMP, 2, &program) }, 0);
}

#[test]
fn real_clone_denial_preserves_errno_and_has_no_fallback_or_birth_observer() {
    isolated(
        "native_child::linux_atomic_launcher_failure_tests::real_clone_denial_preserves_errno_and_has_no_fallback_or_birth_observer",
        || {
            let fixture = AtomicLauncherFixture::new();
            let end = deadline();
            let mut unwind_owner = None;
            let launcher = fixture.launcher("image");
            deny_syscall(libc::SYS_clone);
            let seen = Cell::new(false);
            let result =
                launcher.spawn_observed(end, &mut || check(end), &mut unwind_owner, &mut |_| {
                    seen.set(true);
                    Ok(())
                });
            assert_eq!(original_errno(result), libc::EACCES);
            assert!(!seen.get(), "zero successful kernel birth");
        },
    );
}

#[test]
fn actual_close_range_error_is_typed_and_child_is_reaped_without_exec() {
    isolated(
        "native_child::linux_atomic_launcher_failure_tests::actual_close_range_error_is_typed_and_child_is_reaped_without_exec",
        || {
            let fixture = AtomicLauncherFixture::new();
            let end = deadline();
            let mut unwind_owner = None;
            let launcher = fixture.launcher("image");
            deny_syscall(libc::SYS_close_range);
            let original = RefCell::new(None);
            let result =
                launcher.spawn_observed(end, &mut || check(end), &mut unwind_owner, &mut |child| {
                    *original.borrow_mut() = Some(duplicate(child));
                    Ok(())
                });
            assert_eq!(original_errno(result), libc::EACCES);
            assert_reaped(original.borrow().as_ref().unwrap());
        },
    );
}

// 外层真实thread seccomp先拒绝原syscall；不替换C算法或注入启动错误。
fn assert_private_identity_errno(number: libc::c_long, test: &str) {
    isolated(test, || {
        let fixture = AtomicLauncherFixture::new();
        let end = deadline();
        let launcher = fixture.launcher("image");
        let original = RefCell::new(None);
        let born = Cell::new(false);
        let mut unwind_owner = None;
        deny_syscall(number);
        let result =
            launcher.spawn_observed(end, &mut || check(end), &mut unwind_owner, &mut |child| {
                *original.borrow_mut() = Some(duplicate(child));
                born.set(true);
                Ok(())
            });
        assert!(
            born.get(),
            "original kernel pidfd exists before identity failure"
        );
        assert_eq!(
            original_errno(result),
            libc::EACCES,
            "actual denied syscall errno must not become an identity mismatch"
        );
        assert!(unwind_owner.is_none());
        assert_reaped(original.borrow().as_ref().unwrap());
    });
}

#[test]
fn actual_getpgid_denial_preserves_original_errno_and_reaps() {
    assert_private_identity_errno(
        libc::SYS_getpgid,
        "native_child::linux_atomic_launcher_failure_tests::actual_getpgid_denial_preserves_original_errno_and_reaps",
    );
}

#[test]
fn actual_getsid_denial_preserves_original_errno_and_reaps() {
    assert_private_identity_errno(
        libc::SYS_getsid,
        "native_child::linux_atomic_launcher_failure_tests::actual_getsid_denial_preserves_original_errno_and_reaps",
    );
}

#[test]
fn startup_error_before_init_send_preserves_queued_errno_and_original_wait() {
    isolated(
        "native_child::linux_atomic_launcher_failure_tests::startup_error_before_init_send_preserves_queued_errno_and_original_wait",
        || {
            let fixture = AtomicLauncherFixture::new();
            let end = deadline();
            let launcher = fixture.launcher("image");
            let original = RefCell::new(None);
            let exited = Cell::new(false);
            let mut unwind_owner = None;
            deny_syscall(libc::SYS_close_range);
            let result =
                launcher.spawn_observed(end, &mut || check(end), &mut unwind_owner, &mut |child| {
                    *original.borrow_mut() = Some(duplicate(child));
                    // Init尚未发送；只观察whole-group物理退出，不提前consume原wait。
                    while !child.physically_exited()? {
                        check(end)?;
                        std::thread::yield_now();
                    }
                    exited.set(true);
                    assert!(!child.reaped());
                    Ok(())
                });
            assert!(
                exited.get(),
                "child failed before the real parent Init send"
            );
            assert_eq!(
                original_errno(result),
                libc::EACCES,
                "queued child failure cannot be replaced by parent EPIPE/ECONNRESET"
            );
            assert!(unwind_owner.is_none());
            assert_reaped(original.borrow().as_ref().unwrap());
        },
    );
}
