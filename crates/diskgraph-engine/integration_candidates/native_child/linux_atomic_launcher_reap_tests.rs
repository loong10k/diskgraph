//! 真实线程局部 waitid 拒绝时，物理退出不能替代原 owner 的 wait 消费。

use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_launcher_failure_tests::deny_syscall;
use super::linux_atomic_launcher_fixture::AtomicLauncherFixture;
use super::linux_atomic_launcher_test_support::{check, deadline, isolated};
use super::linux_atomic_reaper_fixture::AtomicReaperFixture;
use super::{ChildError, ChildSpawnError};
use std::cell::{Cell, RefCell};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

fn stopped_before_ready(child: &LinuxAtomicChild, end: Instant) -> Result<(), ChildError> {
    if unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            child.pidfd_for_test(),
            libc::SIGKILL,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    } < 0
    {
        return Err(ChildError::io(
            "test stop original before Ready",
            io::Error::last_os_error(),
        ));
    }
    while !child.physically_exited()? {
        check(end)?;
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn has_errno(error: &ChildError, expected: i32) -> bool {
    match error {
        ChildError::NativeIo { source, .. } => source.raw_os_error() == Some(expected),
        ChildError::Cleanup { primary, cleanup } => {
            has_errno(primary, expected) || has_errno(cleanup, expected)
        }
        _ => false,
    }
}

fn raw_wait_denial(child: &LinuxAtomicChild) -> (i32, Option<i32>) {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            child.pidfd_for_test() as u32,
            &mut info,
            libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
        )
    };
    (
        result,
        if result < 0 {
            io::Error::last_os_error().raw_os_error()
        } else {
            None
        },
    )
}

#[test]
fn waitid_denied_after_physical_exit_returns_original_owner_for_actual_reap() {
    isolated(
        "native_child::linux_atomic_launcher_reap_tests::waitid_denied_after_physical_exit_returns_original_owner_for_actual_reap",
        || {
            let fixture = AtomicLauncherFixture::new();
            let launcher = fixture.launcher("image");
            let end = deadline();
            let reaper = RefCell::new(AtomicReaperFixture::new());
            let observed = Cell::new(false);
            let raw_denial = Cell::new(None);
            let mut unwind_owner = None;
            deny_syscall(libc::SYS_waitid);
            let result =
                launcher.spawn_observed(end, &mut || check(end), &mut unwind_owner, &mut |child| {
                    reaper.borrow_mut().remember(child);
                    stopped_before_ready(child, end)?;
                    raw_denial.set(Some(raw_wait_denial(child)));
                    observed.set(true);
                    Err(ChildError::io(
                        "original birth observer failure",
                        io::Error::from_raw_os_error(libc::ENOMSG),
                    ))
                });
            let (error, owner) = match result {
                Err(failure) => {
                    let (error, owner) = failure.into_parts();
                    (Some(error), owner)
                }
                Ok(child) => (None, Some(child)),
            };
            let physical = owner
                .as_ref()
                .map(|child| child.physically_exited().unwrap());
            let consumed = owner.as_ref().map(LinuxAtomicChild::reaped);
            // 先由出生前的未过滤线程消费所有真实材料，再做目标断言，失败候选也不遗弃 child。
            let retained = reaper.into_inner().finish(owner);
            assert!(observed.get(), "actual original pidfd POLLIN before Ready");
            assert_eq!(
                raw_denial.get(),
                Some((-1, Some(libc::EACCES))),
                "real waitid denial before observer error"
            );
            assert!(
                retained,
                "physical exit is not permission to lose unreaped owner"
            );
            assert_eq!(physical, Some(true));
            assert_eq!(consumed, Some(false));
            assert!(
                unwind_owner.is_none(),
                "normal error uses failure owner, not unwind slot"
            );
            match error.expect("original observer error preserved") {
                ChildSpawnError::Operation(error) => {
                    assert!(has_errno(&error, libc::ENOMSG), "original primary errno");
                    assert!(
                        has_errno(&error, libc::EACCES),
                        "actual cleanup waitid denial"
                    );
                }
                ChildSpawnError::Checkpoint { .. } => {
                    panic!("native observer error changed classification")
                }
            }
        },
    );
}

#[test]
fn waitid_denied_unwind_keeps_original_box_and_transfers_unreaped_owner() {
    isolated(
        "native_child::linux_atomic_launcher_reap_tests::waitid_denied_unwind_keeps_original_box_and_transfers_unreaped_owner",
        || {
            let fixture = AtomicLauncherFixture::new();
            let launcher = fixture.launcher("image");
            let end = deadline();
            let reaper = RefCell::new(AtomicReaperFixture::new());
            let observed = Cell::new(false);
            let raw_denial = Cell::new(None);
            let mut sentinel = Some(Box::new(0x42_7069_6466_u64));
            let identity = (&**sentinel.as_ref().unwrap()) as *const u64 as usize;
            let mut unwind_owner = None;
            // 仅本隔离测试的预期 panic 限量诊断；不改变被恢复的 payload 或生产回收流程。
            let original_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| eprintln!("DG_ATOMIC_EXPECTED_UNWIND")));
            deny_syscall(libc::SYS_waitid);
            let caught = catch_unwind(AssertUnwindSafe(|| {
                launcher.spawn_observed(end, &mut || check(end), &mut unwind_owner, &mut |child| {
                    reaper.borrow_mut().remember(child);
                    stopped_before_ready(child, end)?;
                    raw_denial.set(Some(raw_wait_denial(child)));
                    observed.set(true);
                    std::panic::panic_any(sentinel.take().unwrap());
                })
            }));
            std::panic::set_hook(original_hook);
            let (payload, unexpected_owner) = match caught {
                Err(payload) => (Some(payload), None),
                Ok(Err(failure)) => {
                    let (_, owner) = failure.into_parts();
                    (None, owner)
                }
                Ok(Ok(child)) => (None, Some(child)),
            };
            let in_external_slot = unwind_owner.is_some();
            let owner = unwind_owner.take().or(unexpected_owner);
            let physical = owner
                .as_ref()
                .map(|child| child.physically_exited().unwrap());
            let consumed = owner.as_ref().map(LinuxAtomicChild::reaped);
            let retained = reaper.into_inner().finish(owner);
            assert!(
                observed.get(),
                "actual whole group exit before specified panic"
            );
            assert_eq!(
                raw_denial.get(),
                Some((-1, Some(libc::EACCES))),
                "real waitid denial before sentinel panic"
            );
            assert!(
                in_external_slot && retained,
                "only external finite slot retains original owner on unwind"
            );
            assert_eq!(physical, Some(true));
            assert_eq!(consumed, Some(false));
            let payload = payload.expect("original ordinary Rust panic propagated");
            let original = payload
                .downcast_ref::<Box<u64>>()
                .expect("unchanged Box payload type");
            assert_eq!(
                (&**original) as *const u64 as usize,
                identity,
                "exact original Box identity"
            );
            assert_eq!(**original, 0x42_7069_6466);
        },
    );
}
