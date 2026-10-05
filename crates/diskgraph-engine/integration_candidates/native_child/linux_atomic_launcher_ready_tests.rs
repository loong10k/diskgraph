//! 用真实 EOF 与原 pidfd 固定 waitid/POLLIN 间的退出，不伪造 syscall 或概率等待。
use super::ChildSpawnError;
use super::linux_atomic_launcher_fixture::AtomicLauncherFixture;
use super::linux_atomic_launcher_test_support::{assert_reaped, check, deadline, duplicate};
use std::cell::{Cell, RefCell};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;
use std::time::Instant;

type Gap = Box<dyn FnOnce(Option<i32>, Option<i32>)>;
thread_local! {
    static READY_GAP: RefCell<Option<Gap>> = const { RefCell::new(None) };
}

/// 参数：两个缓存值来自本次真实非阻塞 waitid；返回：仅运行本线程一次调度动作。
pub(super) fn after_observe(code: Option<i32>, signal: Option<i32>) {
    let hook = READY_GAP.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook(code, signal);
    }
}

fn with_gap<T>(hook: Gap, work: impl FnOnce() -> T) -> T {
    /// 即使工作或观察点 panic，也清除本线程的测试动作，不能污染后续用例。
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            READY_GAP.with(|slot| slot.borrow_mut().take());
        }
    }
    READY_GAP.with(|slot| {
        assert!(slot.borrow().is_none(), "no nested scheduling hook");
        *slot.borrow_mut() = Some(hook);
    });
    let _reset = Reset;
    work()
}

fn wait_natural_exit(pidfd: &OwnedFd, pid: i32, end: Instant) {
    let mut view = libc::pollfd {
        fd: pidfd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        check(end).unwrap();
        let remaining = end.saturating_duration_since(Instant::now());
        let timeout = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(&mut view, 1, timeout) };
        if result < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        assert_eq!(
            result, 1,
            "original pidfd becomes readable within original deadline"
        );
        assert_eq!(view.revents & (libc::POLLERR | libc::POLLNVAL), 0);
        assert_ne!(view.revents & libc::POLLIN, 0);
        break;
    }
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    loop {
        check(end).unwrap();
        let result = unsafe {
            libc::waitid(
                libc::P_PIDFD,
                pidfd.as_raw_fd() as u32,
                &mut info,
                libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
            )
        };
        if result < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        assert_eq!(result, 0, "nonconsuming original wait observation");
        break;
    }
    assert_eq!(unsafe { info.si_pid() }, pid);
    assert_eq!(info.si_code, libc::CLD_EXITED);
    assert_eq!(unsafe { info.si_status() }, 127);
}

#[test]
fn exit_between_initial_waitid_and_poll_preserves_natural_exit_record() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let original = RefCell::new(None);
    let gap_seen = Rc::new(Cell::new(false));
    let observed = Cell::new(None);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        fixture.launcher("image").spawn_observed(
            end,
            &mut || check(end),
            &mut unwind_owner,
            &mut |child| {
                *original.borrow_mut() = Some(duplicate(child));
                // 保留最后一个父端 startup 引用；首次真实 WNOHANG 前 child 不可能收到 EOF。
                let fd = unsafe { libc::fcntl(child.startup_fd()?, libc::F_DUPFD_CLOEXEC, 3) };
                assert!(fd >= 0, "duplicate actual startup endpoint");
                let held_startup = unsafe { OwnedFd::from_raw_fd(fd) };
                child.close_startup_for_test();
                let pidfd = duplicate(child);
                let pid = child.pid_for_test();
                let seen = Rc::clone(&gap_seen);
                let ready = with_gap(
                    Box::new(move |code, signal| {
                        assert_eq!((code, signal), (None, None), "first wait saw no exit");
                        drop(held_startup);
                        wait_natural_exit(&pidfd, pid, end);
                        seen.set(true);
                    }),
                    || child.poll(),
                )?;
                observed.set(Some((
                    ready,
                    child.exit_code(),
                    child.exit_signal(),
                    child.reaped(),
                )));
                Ok(())
            },
        )
    }));
    // 任意观察点 panic 先经过 launcher 清理；保留 owner 时仍用原 owner 实际回收。
    if let Some(mut owner) = unwind_owner.take() {
        owner.cleanup().unwrap();
    }
    let failed_as_expected = match caught {
        Err(payload) => {
            if let Some(pidfd) = original.borrow().as_ref() {
                assert_reaped(pidfd);
            }
            resume_unwind(payload);
        }
        Ok(Err(failure)) => {
            let (error, owner) = failure.into_parts();
            if let Some(mut owner) = owner {
                owner.cleanup().unwrap();
            }
            matches!(error, ChildSpawnError::Operation(_))
        }
        Ok(Ok(mut child)) => {
            child.cleanup().unwrap();
            false
        }
    };
    assert_reaped(original.borrow().as_ref().expect("original birth observed"));
    assert!(
        gap_seen.get(),
        "actual EOF exit was forced between original wait and poll"
    );
    assert!(
        failed_as_expected,
        "closed startup never becomes successful launch"
    );
    eprintln!(
        "DG_ATOMIC_READY_GAP actual_exit=127 nonconsuming_wait=true observed={:?}",
        observed.get()
    );
    assert_eq!(observed.get(), Some((true, Some(127), None, false)));
}
