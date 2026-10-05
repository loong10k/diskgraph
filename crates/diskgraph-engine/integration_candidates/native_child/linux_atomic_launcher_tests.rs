//! 新 Rust launcher 的真实镜像、线程组和传输验收；不以旧 C 机制结果代替。

use super::ControlWriteStatus;
use super::linux_atomic_launcher_fixture::AtomicLauncherFixture;
use super::linux_atomic_launcher_test_support::{
    assert_reaped, check, complete, deadline, duplicate, isolated, line, write,
};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[test]
fn held_elf_replacement_preserves_image_and_closes_unrelated_descriptors() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let path = fixture.directory().join("held-image");
    std::fs::copy(fixture.binary(), &path).unwrap();
    let image = File::open(&path).unwrap();
    let canary_file = File::open("/dev/null").unwrap();
    let raw = unsafe { libc::fcntl(canary_file.as_raw_fd(), libc::F_DUPFD, 128) };
    assert!(raw >= 128);
    let canary = unsafe { OwnedFd::from_raw_fd(raw) };
    let launcher = AtomicLauncherFixture::from_file(image, "image", Some(canary.as_raw_fd()));
    let replaced = fixture.directory().join("replacement");
    std::fs::copy(fixture.replacement(), &replaced).unwrap();
    std::fs::rename(&replaced, &path).unwrap();
    let mut child = launcher
        .spawn(end, &mut || check(end), &mut unwind_owner)
        .unwrap();
    let original = duplicate(&child);
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    let (out, err) = complete(&mut child, end);
    assert_eq!(line(&out, "IMAGE="), "1");
    assert_eq!(line(&out, "CANARY_CLOSED="), "1");
    assert_eq!(line(&out, "FIXED="), "sentinel");
    assert_eq!(line(&out, "PATH_ABSENT="), "1");
    assert_eq!(line(&out, "LOADER_ABSENT="), "1");
    assert_eq!(line(&err, "STDERR_IMAGE="), "1");
    assert_eq!(child.exit_code(), Some(0));
    assert_reaped(&original);
    // 第二份 ELF 也从同一真实生产入口执行，不能仅以 artifact 名称推断 image ID。
    let replacement =
        AtomicLauncherFixture::from_file(File::open(fixture.replacement()).unwrap(), "image", None);
    let mut second = replacement
        .spawn(end, &mut || check(end), &mut unwind_owner)
        .unwrap();
    second.request_control_close().unwrap();
    assert_eq!(line(&complete(&mut second, end).0, "IMAGE="), "2");
}

#[test]
fn production_stdin_fragments_and_real_eof_precede_original_pidfd_wait() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let mut child = fixture
        .launcher("echo")
        .spawn(end, &mut || check(end), &mut unwind_owner)
        .unwrap();
    let original = duplicate(&child);
    assert!(!child.poll().unwrap(), "waiting for real stdin");
    let bytes = [0_u8, 128, 255, 13, 10, 1, 200];
    write(&mut child, &bytes[..3], end);
    write(&mut child, &bytes[3..], end);
    child.request_control_close().unwrap();
    let (out, err) = complete(&mut child, end);
    assert!(err.is_empty());
    assert_eq!(line(&out, "HEX="), hex::encode(bytes));
    assert_eq!(line(&out, "STDIN_EOF="), "true");
    assert_eq!(child.exit_code(), Some(0));
    assert_reaped(&original);
}

#[test]
fn scanner_filter_rejects_process_clone_but_actual_pthread_still_runs() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let mut child = fixture
        .launcher("derive")
        .spawn(end, &mut || check(end), &mut unwind_owner)
        .unwrap();
    let original = duplicate(&child);
    child.request_control_close().unwrap();
    let (out, err) = complete(&mut child, end);
    assert!(err.is_empty());
    assert_eq!(line(&out, "PROCESS_CLONE="), "-1");
    assert_eq!(line(&out, "PROCESS_CLONE_ERRNO="), libc::EPERM.to_string());
    assert_eq!(line(&out, "SETSID="), "-1");
    assert_eq!(line(&out, "SETSID_ERRNO="), libc::EPERM.to_string());
    assert_eq!(line(&out, "SETPGID="), "-1");
    assert_eq!(line(&out, "SETPGID_ERRNO="), libc::EPERM.to_string());
    assert_eq!(line(&out, "PTHREAD_CREATE="), "0");
    assert!(String::from_utf8(out).unwrap().contains("THREAD_HEARTBEAT"));
    assert_eq!(child.exit_code(), Some(0));
    assert_reaped(&original);
}

#[test]
fn main_pthread_exit_is_not_whole_thread_group_exit() {
    let fixture = AtomicLauncherFixture::new();
    let end = deadline();
    let mut unwind_owner = None;
    let mut child = fixture
        .launcher("thread_exit")
        .spawn(end, &mut || check(end), &mut unwind_owner)
        .unwrap();
    let original = duplicate(&child);
    let mut out = Vec::new();
    loop {
        check(end).unwrap();
        if let Some(bytes) = child.read_stdout().unwrap() {
            assert!(bytes.len() <= 4096 - out.len());
            out.extend_from_slice(bytes);
        }
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.pid_for_test())).unwrap();
        let state = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .next()
            .unwrap();
        if state == "Z" && String::from_utf8_lossy(&out).contains("THREAD_HEARTBEAT") {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(String::from_utf8_lossy(&out).contains("MAIN_PTHREAD_EXIT"));
    assert!(
        !child.poll().unwrap(),
        "original pidfd still owns live worker threads"
    );
    assert!(!child.poll_normal_exit(&mut || check(end)).unwrap());
    child.request_control_close().unwrap();
    complete(&mut child, end);
    assert_eq!(child.exit_code(), Some(0));
    assert_reaped(&original);
}

static HOST_HANDLER: AtomicUsize = AtomicUsize::new(0);
static ATFORK_CALLS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn host_handler(_: libc::c_int) {
    HOST_HANDLER.fetch_add(1, Ordering::SeqCst);
}
extern "C" fn atfork_hook() {
    ATFORK_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn multithreaded_host_handlers_and_atfork_do_not_run_in_child_branch() {
    isolated(
        "native_child::linux_atomic_launcher_tests::multithreaded_host_handlers_and_atfork_do_not_run_in_child_branch",
        || {
            let fixture = AtomicLauncherFixture::new();
            let end = deadline();
            let mut unwind_owner = None;
            assert_eq!(
                unsafe {
                    libc::pthread_atfork(Some(atfork_hook), Some(atfork_hook), Some(atfork_hook))
                },
                0
            );
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = host_handler as usize;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            assert_eq!(
                unsafe { libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()) },
                0
            );
            let mut before: libc::sigset_t = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut before) },
                0
            );
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let barrier = std::sync::Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        barrier.wait();
                    })
                })
                .collect();
            barrier.wait();
            let mut child = fixture
                .launcher("signal")
                .spawn(end, &mut || check(end), &mut unwind_owner)
                .unwrap();
            child.request_control_close().unwrap();
            let (out, _) = complete(&mut child, end);
            assert!(String::from_utf8_lossy(&out).contains("BEFORE_DEFAULT_SIGNAL"));
            assert!(!String::from_utf8_lossy(&out).contains("INHERITED_HANDLER_WAS_USED"));
            assert_eq!(child.exit_signal(), Some(libc::SIGUSR1));
            assert_eq!(HOST_HANDLER.load(Ordering::SeqCst), 0);
            assert_eq!(ATFORK_CALLS.load(Ordering::SeqCst), 0);
            assert_eq!(unsafe { libc::raise(libc::SIGUSR1) }, 0);
            assert_eq!(
                HOST_HANDLER.load(Ordering::SeqCst),
                1,
                "parent disposition unchanged"
            );
            let mut after: libc::sigset_t = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut after) },
                0
            );
            for signal in 1..=64 {
                assert_eq!(unsafe { libc::sigismember(&before, signal) }, unsafe {
                    libc::sigismember(&after, signal)
                });
            }
            barrier.wait();
            for thread in threads {
                thread.join().unwrap();
            }
            assert!(Instant::now() < end);
        },
    );
}
