//! 正常退出测试的有限观察；来源：真实 Unix 子进程、WNOWAIT 与非阻塞管道。

use super::UnixChild;
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

/// 使用 fresh 工厂启动固定 fixture，不允许调用方注入 pre_exec。
/// 参数：mode 为固定测试模式，directory 为隔离目录；返回：独占 child owner。
pub(super) fn spawn(mode: &str, directory: &Path) -> UnixChild {
    UnixChild::spawn_worker(
        &std::env::current_exe().unwrap(),
        &[
            OsStr::new("--exact"),
            OsStr::new("native_child::unix_normal_exit_fixture::normal_fixture"),
            OsStr::new("--nocapture"),
        ],
        &[
            (OsStr::new("DG_NORMAL_FIXTURE"), OsStr::new(mode)),
            (OsStr::new("DG_NORMAL_DIRECTORY"), directory.as_os_str()),
        ],
        || Ok::<(), ()>(()),
    )
    .unwrap()
}

/// 等待真实文件标记；参数：path 为本次 fixture 路径；返回：观察到标记时正常返回。
pub(super) fn wait_file(path: &Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "missing {path:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 读取真实 fixture 标量；参数：directory、name 指定标记；返回：系统实际身份或序号。
pub(super) fn scalar(directory: &Path, name: &str) -> i32 {
    std::fs::read_to_string(directory.join(name))
        .unwrap()
        .parse()
        .unwrap()
}

/// 验证 fresh 入口创建独立 session；参数：directory 为 leader 标记；返回：实际 leader PID。
pub(super) fn qualified_identity(directory: &Path) -> i32 {
    wait_file(&directory.join("ready"));
    let pid = scalar(directory, "pid");
    assert_eq!(scalar(directory, "pgid"), pid);
    assert_eq!(scalar(directory, "sid"), pid);
    assert_eq!(scalar(directory, "ppid"), std::process::id() as i32);
    // 安全性：仅查询当前宿主 session；不改变任何 session/group。
    assert_ne!(pid, unsafe { libc::getsid(0) });
    pid
}

/// 排空原双管道，可选择只保留已退出 leader，绝不调用 cleanup。
/// 参数：child 为原 owner，wait_exit 指定是否要求 WNOWAIT 已观察退出；返回：原 stdout。
pub(super) fn drain(child: &mut UnixChild, wait_exit: bool) -> Vec<u8> {
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
        assert!(stdout.len() + stderr.len() < 64 * 1024);
        let exited = child.poll().unwrap();
        if child.stdout_eof() && child.stderr_eof() && (!wait_exit || exited) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "pipe/exit watchdog {stderr:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        std::str::from_utf8(&stdout)
            .unwrap()
            .contains("DG_NORMAL_END=true")
    );
    stdout
}

/// 验证 leader 的真实退出状态仍未被回收；参数：pid 为本测试 leader；返回：未保留时断言失败。
pub(super) fn retained(pid: i32) {
    // 安全性：输出可零初始化；只观察本测试 PID，WNOWAIT 不消费状态。
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            libc::waitid(
                libc::P_PID,
                pid as u32,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        },
        0
    );
    assert_eq!(unsafe { info.si_pid() }, pid);
    assert_eq!(info.si_code, libc::CLD_EXITED);
}

/// 观察 heartbeat 实际推进；参数：directory 为活进程标记；返回：不能推进时断言失败。
pub(super) fn advancing(directory: &Path) {
    wait_file(&directory.join("heartbeat"));
    let prior = scalar(directory, "heartbeat");
    let started = Instant::now();
    while scalar(directory, "heartbeat") <= prior {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "heartbeat stopped"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 自然释放 fixture；参数：directory 为 leader 或普通后代目录；返回：真实完成标记已存在。
pub(super) fn release(directory: &Path) {
    std::fs::write(directory.join("release"), b"release").unwrap();
    wait_file(&directory.join("naturally_finished"));
}

/// 借同一调用方检查轮询正常许可，绝不 SIGKILL；参数：child 为原 owner；返回：实际 normal/reap 后检查次数。
#[cfg(target_os = "macos")]
pub(super) fn finish(child: &mut UnixChild) -> usize {
    let started = Instant::now();
    let mut calls = 0;
    loop {
        let finished = child
            .poll_normal_exit(|| {
                calls += 1;
                assert!(
                    started.elapsed() < Duration::from_secs(20),
                    "normal exit watchdog"
                );
                Ok::<(), ()>(())
            })
            .unwrap();
        if finished {
            return calls;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 证明 owner 已实际 wait；参数：pid 为本测试 leader；返回：未回收时断言失败。
pub(super) fn reaped(pid: i32) {
    let mut status = 0;
    // 安全性：仅验证本测试已退出 PID 不再是可 wait 子进程，不使用 wait(-1)。
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}
