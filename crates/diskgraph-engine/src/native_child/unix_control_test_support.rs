//! Unix 控制输入测试的真实进程与有限观察辅助；来源：原生 Rust OS-child 测试。

use super::{ChildInputMode, ControlWriteStatus, UnixChild};
use std::process::Command;
use std::time::{Duration, Instant};

/// 构造当前测试可执行文件的真实子进程。参数：mode 为固定模式，directory 为隔离目录；返回：结构化命令。
pub(super) fn command(mode: &str, directory: &std::path::Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "native_child::unix_control_fixture::stdin_fixture",
            "--nocapture",
        ])
        .env("DG_CONTROL_FIXTURE", mode)
        .env("DG_CONTROL_DIRECTORY", directory);
    command
}

/// 使用实际控制输入启动 fixture。参数：mode、directory 为测试配置；返回：唯一原生 child owner。
pub(super) fn spawn(mode: &str, directory: &std::path::Path) -> UnixChild {
    UnixChild::spawn_with_input(
        &mut command(mode, directory),
        ChildInputMode::WorkerControl,
        || Ok::<(), ()>(()),
    )
    .unwrap()
}

/// 等待 fixture 写入阶段标记。参数：path 为本测试标记；返回：仅已观察到真实文件时正常返回。
pub(super) fn wait_file(path: &std::path::Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "missing {path:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 按真实完成字节推进分片。参数：child 为原 owner，bytes 为原数据；返回：调用方观察检查次数。
pub(super) fn write_bytes(child: &mut UnixChild, bytes: &[u8]) -> usize {
    let started = Instant::now();
    let mut offset = 0;
    let mut checkpoints = 0;
    while offset < bytes.len() {
        checkpoints += 1;
        assert!(started.elapsed() < Duration::from_secs(20));
        let end = (offset + ControlWriteStatus::MAX_CHUNK_BYTES).min(bytes.len());
        let mut status = child.start_control_write(&bytes[offset..end]).unwrap();
        while status == ControlWriteStatus::Pending {
            checkpoints += 1;
            assert!(started.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(1));
            status = child.poll_control_write().unwrap();
        }
        match status {
            ControlWriteStatus::Written(count) => {
                assert!(count > 0 && count <= end - offset);
                offset += count;
            }
            other => panic!("unexpected write status: {other:?}"),
        }
    }
    checkpoints
}

/// 排空双管道并沿旧 cleanup 实际回收。参数：child、directory 为隔离 fixture；返回：原始 stdout。
pub(super) fn complete(child: &mut UnixChild, directory: &std::path::Path) -> Vec<u8> {
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
        assert!(stdout.len() + stderr.len() < 1024 * 1024);
        if child.poll().unwrap() && child.stdout_eof() && child.stderr_eof() {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "child did not exit: {}",
            String::from_utf8_lossy(&stderr)
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(child.exit_code(), Some(0), "{stderr:?}");
    // 此处沿用终止组+真实 wait 的旧 cleanup；不是正常整组空许可。
    child.cleanup().unwrap();
    assert_reaped(directory);
    stdout
}

/// 验证本测试 leader 已被 owner 回收。参数：directory 为 PID 标记目录；返回：未回收时断言失败。
pub(super) fn assert_reaped(directory: &std::path::Path) {
    let pid: i32 = std::fs::read_to_string(directory.join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    let mut status = 0;
    // 安全性：PID 来自本测试子进程；仅确认 owner 已执行 wait，不自行回收其它子进程。
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

/// 从真实输出取固定标记。参数：stdout 为原始字节，prefix 为标记；返回：该行的借用值。
pub(super) fn line<'a>(stdout: &'a [u8], prefix: &str) -> &'a str {
    std::str::from_utf8(stdout)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix} in {stdout:?}"))
}
