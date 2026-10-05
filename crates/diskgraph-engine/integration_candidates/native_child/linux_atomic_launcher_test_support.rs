//! 生产 pidfd owner 的真实管道、退出与隔离宿主测试支持，不复刻出生算法。

use super::linux_atomic_child::LinuxAtomicChild;
use super::{ChildError, ControlWriteStatus, UnixChild};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::process::Command;
use std::time::{Duration, Instant};

pub(super) fn deadline() -> Instant {
    Instant::now().checked_add(Duration::from_secs(20)).unwrap()
}

pub(super) fn check(deadline: Instant) -> Result<(), ChildError> {
    if Instant::now() >= deadline {
        return Err(ChildError::Unsupported(
            "atomic launcher test deadline exceeded",
        ));
    }
    Ok(())
}

pub(super) fn write(child: &mut LinuxAtomicChild, bytes: &[u8], end: Instant) {
    let mut position = 0;
    let mut pending = false;
    while position < bytes.len() {
        check(end).unwrap();
        let status = if pending {
            child.poll_control_write().unwrap()
        } else {
            let length = (bytes.len() - position).min(ControlWriteStatus::MAX_CHUNK_BYTES);
            child
                .start_control_write(&bytes[position..position + length])
                .unwrap()
        };
        match status {
            ControlWriteStatus::Written(count) => {
                assert!(count > 0 && count <= bytes.len() - position);
                position += count;
                pending = false;
            }
            ControlWriteStatus::Pending => pending = true,
            ControlWriteStatus::Closed => panic!("premature control close"),
        }
        if pending {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

pub(super) fn complete(child: &mut LinuxAtomicChild, end: Instant) -> (Vec<u8>, Vec<u8>) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    loop {
        check(end).unwrap();
        if let Some(bytes) = child.read_stdout().unwrap() {
            append(&mut out, bytes);
        }
        if let Some(bytes) = child.read_stderr().unwrap() {
            append(&mut err, bytes);
        }
        if child.poll_normal_exit(&mut || check(end)).unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(child.stdout_eof() && child.stderr_eof());
    assert!(
        child.poll_normal_exit(&mut || check(end)).unwrap(),
        "cached wait"
    );
    (out, err)
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) {
    assert!(bytes.len() <= 4096 - output.len(), "bounded fixture output");
    output.extend_from_slice(bytes);
}

pub(super) fn line(bytes: &[u8], key: &str) -> String {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(key).map(str::to_owned))
        .unwrap_or_else(|| panic!("missing {key} in {bytes:?}"))
}

pub(super) fn duplicate(child: &LinuxAtomicChild) -> OwnedFd {
    let fd = unsafe { libc::fcntl(child.pidfd_for_test(), libc::F_DUPFD_CLOEXEC, 3) };
    assert!(fd >= 0);
    unsafe { OwnedFd::from_raw_fd(fd) }
}

pub(super) fn assert_reaped(pidfd: &OwnedFd) {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PIDFD,
            pidfd.as_raw_fd() as u32,
            &mut info,
            libc::WEXITED | libc::WNOHANG,
        )
    };
    assert_eq!(result, -1);
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    let mut poll = libc::pollfd {
        fd: pidfd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    assert_eq!(unsafe { libc::poll(&mut poll, 1, 0) }, 1);
    assert!(
        poll.revents & libc::POLLIN != 0,
        "original whole thread group exited"
    );
}

/// 普通 Rust 测试函数在隔离的真实宿主内运行；Null retained group兜底回收异常后代。
pub(super) fn isolated(name: &str, body: impl FnOnce()) {
    if std::env::var("DG_ATOMIC_LAUNCH_HOST").as_deref() == Ok(name) {
        body();
        return;
    }
    let end = deadline();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env("DG_ATOMIC_LAUNCH_HOST", name);
    let mut host = UnixChild::spawn(&mut command, || check(end)).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let observed = catch_unwind(AssertUnwindSafe(|| {
        loop {
            check(end).unwrap();
            if let Some(bytes) = host.read_stdout().unwrap() {
                append(&mut out, bytes);
            }
            if let Some(bytes) = host.read_stderr().unwrap() {
                append(&mut err, bytes);
            }
            if host.poll().unwrap() && host.stdout_eof() && host.stderr_eof() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        host.exit_code()
    }));
    let cleanup = host.cleanup();
    match observed {
        Ok(code) => {
            cleanup.unwrap();
            assert_eq!(
                code,
                Some(0),
                "isolated actual host out={out:?} err={err:?}"
            );
        }
        Err(payload) => {
            eprintln!("atomic launcher isolated-host cleanup={cleanup:?}");
            resume_unwind(payload);
        }
    }
}
