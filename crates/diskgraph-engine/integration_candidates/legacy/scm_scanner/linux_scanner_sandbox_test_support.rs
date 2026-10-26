//! 受限扫描入口的真实 OS 观察；来源：Linux pidfd、WNOWAIT 与独占测试子进程。

use super::linux_atomic_birth_fixture::AtomicBirthFixture;
use super::UnixChild;
use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 读取 root 明确编译的 C artifact；参数：无；返回：绝对路径，缺失明确失败。
pub(super) fn binary() -> PathBuf {
    let path = match std::env::var_os("DG_LINUX_SCANNER_SANDBOX_BINARY") {
        Some(path) => PathBuf::from(path),
        None => panic!("root must compile and provide DG_LINUX_SCANNER_SANDBOX_BINARY explicitly"),
    };
    assert!(path.is_absolute() && path.is_file(), "qualified C artifact");
    path
}

/// 调用真实受限入口；参数：mode、directory 为固定夹具输入；返回：唯一 child owner。
pub(super) fn spawn(mode: &str, directory: &Path) -> UnixChild {
    UnixChild::spawn_scanner(
        &binary(),
        &[OsStr::new(mode), directory.as_os_str()],
        &[],
        || Ok::<(), ()>(()),
    )
    .unwrap()
}

/// 观察真实标记；参数：directory、name 为隔离路径；返回：存在时结束，不造伪标记。
pub(super) fn wait_file(directory: &Path, name: &str) {
    let started = Instant::now();
    while !directory.join(name).exists() {
        assert!(started.elapsed() < Duration::from_secs(20), "marker {name}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 读取 native 标量；参数：directory、name 指定真实文件；返回：真实系统值。
pub(super) fn scalar(directory: &Path, name: &str) -> i64 {
    wait_file(directory, name);
    std::fs::read_to_string(directory.join(name))
        .unwrap()
        .parse()
        .unwrap()
}

/// 核对 fresh session 与实际过滤器状态；参数：directory 为本次根；返回：原 leader PID。
pub(super) fn identity(directory: &Path) -> i32 {
    wait_file(directory, "ready");
    let pid = i32::try_from(scalar(directory, "pid")).unwrap();
    assert_eq!(scalar(directory, "main_tid"), i64::from(pid));
    assert_eq!(scalar(directory, "pgid"), i64::from(pid));
    assert_eq!(scalar(directory, "sid"), i64::from(pid));
    assert_eq!(scalar(directory, "ppid"), i64::from(std::process::id()));
    assert_eq!(scalar(directory, "no_new_privs"), 1);
    assert_eq!(scalar(directory, "seccomp"), 2);
    pid
}

/// 公平读取双管道直到真实 EOF；参数：child 为原 owner；返回：有界真实 stdout。
pub(super) fn drain(child: &mut UnixChild) -> Vec<u8> {
    let started = Instant::now();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        if let Some(bytes) = child.read_stdout().unwrap() {
            stdout.extend_from_slice(bytes);
        }
        if let Some(bytes) = child.read_stderr().unwrap() {
            stderr.extend_from_slice(bytes);
        }
        assert!(
            stdout.len() + stderr.len() <= 65536,
            "bounded fixture output"
        );
        if child.stdout_eof() && child.stderr_eof() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "EOF {stderr:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(stderr.is_empty(), "native fixture error {stderr:?}");
    stdout
}

/// 借原调用方检查获得正常回收；参数：child 为同一 owner；返回：实际许可后的检查次数。
pub(super) fn finish(child: &mut UnixChild) -> usize {
    let started = Instant::now();
    let mut calls = 0;
    loop {
        let exited = child
            .poll_normal_exit(|| {
                calls += 1;
                assert!(started.elapsed() < Duration::from_secs(20), "normal wait");
                Ok::<(), ()>(())
            })
            .unwrap();
        if exited {
            return calls;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 独立持有真实 whole-process pidfd；参数：pid 为未回收 child；返回：内核句柄。
pub(super) fn pidfd(pid: i32) -> OwnedFd {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(fd >= 0, "pidfd_open {:?}", io::Error::last_os_error());
    // 安全性：成功 syscall 新建本测试唯一 fd，非借用 std Child 的描述符。
    unsafe { OwnedFd::from_raw_fd(i32::try_from(fd).unwrap()) }
}

/// 零等待观察 whole-thread-group exit；参数：fd 为原 pidfd；返回：实际可读状态。
pub(super) fn pidfd_ready(fd: &OwnedFd) -> bool {
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    assert!(unsafe { libc::poll(&mut poll, 1, 0) } >= 0);
    assert_eq!(poll.revents & (libc::POLLERR | libc::POLLNVAL), 0);
    poll.revents & libc::POLLIN != 0
}

/// 证明真实主线程已退出而线程组仍活；参数：pid 为 leader；返回：完整 stat 资格成立。
pub(super) fn main_zombie_with_threads(pid: i32) {
    let started = Instant::now();
    loop {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        assert!(stat.len() < 4096, "bounded stat");
        let tail = &stat[stat.rfind(')').unwrap() + 2..];
        let fields: Vec<_> = tail.split_ascii_whitespace().collect();
        if fields[0] == "Z" && fields[17].parse::<usize>().unwrap() >= 2 {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "main Z/live pthreads"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 观察原 pthread heartbeat 推进；参数：directory、index 指定线程；返回：真实新序号。
pub(super) fn advancing(directory: &Path, index: usize) -> i64 {
    let name = format!("heartbeat_{index}");
    let prior = scalar(directory, &name);
    let started = Instant::now();
    loop {
        let next = scalar(directory, &name);
        if next > prior {
            return next;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "live heartbeat"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 核对本 owner 已实际 wait；参数：pid 为原 child；返回：不可再回收时通过。
pub(super) fn reaped(pid: i32) {
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

/// 只对既有 SCM 生产入口做兼容正控；参数：fixture 为原 C ELF；返回：实际退出结果。
pub(super) fn existing_scanner_route(fixture: &AtomicBirthFixture) -> Vec<u8> {
    let mut child = UnixChild::spawn_scanner(
        fixture.binary(),
        &[OsStr::new("image")],
        &[],
        || Ok::<(), ()>(()),
    )
    .unwrap();
    child.request_control_close().unwrap();
    let started = Instant::now();
    let mut output = Vec::new();
    let mut stderr = Vec::new();
    loop {
        if let Some(bytes) = child.read_stdout().unwrap() {
            output.extend_from_slice(bytes);
        }
        if let Some(bytes) = child.read_stderr().unwrap() {
            stderr.extend_from_slice(bytes);
        }
        assert!(output.len() + stderr.len() <= 4096);
        if child.stdout_eof() && child.stderr_eof() {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    loop {
        if child.poll_normal_exit(|| Ok::<(), ()>(())).unwrap() {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(stderr.is_empty());
    assert_eq!(child.exit_code(), Some(0));
    output
}
