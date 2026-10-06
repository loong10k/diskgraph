//! 正常整组退出的真实 Unix fixture；来源：Rust 测试进程与原生 session/group/wait 语义。

use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn normal_fixture() {
    let Ok(mode) = std::env::var("DG_NORMAL_FIXTURE") else {
        return;
    };
    let directory = std::path::PathBuf::from(std::env::var_os("DG_NORMAL_DIRECTORY").unwrap());
    write_identity(&directory);
    match mode.as_str() {
        "exit" => terminal(),
        "nonzero" => {
            terminal();
            std::process::exit(7);
        }
        "live_end" => {
            terminal();
            close_output();
            heartbeat_until_release(&directory);
        }
        "descendant" => descendant(&directory, 1),
        "two_descendants" => descendant(&directory, 2),
        "leaf" => {
            // 安全性：仅 fixture 关闭自己的标准描述符；父宿主与其它测试不受影响。
            for fd in [0, 1, 2] {
                assert_eq!(unsafe { libc::close(fd) }, 0);
            }
            std::fs::write(directory.join("stdio_closed"), b"closed").unwrap();
            heartbeat_until_release(&directory);
        }
        other => panic!("unknown normal fixture mode {other}"),
    }
    // 输出与业务终态已产生仍不算父侧许可；只有本进程实际退出才停止其所有线程。
    std::process::exit(0);
}

fn write_identity(directory: &Path) {
    std::fs::write(directory.join("pid"), std::process::id().to_string()).unwrap();
    // 安全性：只读取本 fixture 的实际 session、group 与父进程身份。
    std::fs::write(
        directory.join("pgid"),
        unsafe { libc::getpgrp() }.to_string(),
    )
    .unwrap();
    std::fs::write(
        directory.join("sid"),
        unsafe { libc::getsid(0) }.to_string(),
    )
    .unwrap();
    std::fs::write(
        directory.join("ppid"),
        unsafe { libc::getppid() }.to_string(),
    )
    .unwrap();
    std::fs::write(directory.join("ready"), b"ready").unwrap();
}

fn terminal() {
    println!("DG_NORMAL_END=true");
    std::io::stdout().flush().unwrap();
}

fn close_output() {
    // 安全性：仅关闭独立 fixture 自己的管道副本，真实 EOF 由父读取验证。
    for fd in [1, 2] {
        assert_eq!(unsafe { libc::close(fd) }, 0);
    }
}

fn descendant(directory: &Path, count: usize) {
    for index in 0..count {
        let leaf = directory.join(if index == 0 { "leaf" } else { "leaf2" });
        std::fs::create_dir(&leaf).unwrap();
        // 不设置 process_group/pre_exec：普通后代真实继承 leader 的独立 session 和 group。
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "native_child::unix_normal_exit_fixture::normal_fixture",
                "--nocapture",
            ])
            .env("DG_NORMAL_FIXTURE", "leaf")
            .env("DG_NORMAL_DIRECTORY", &leaf)
            .spawn()
            .unwrap();
        wait_file(&leaf.join("stdio_closed"));
        assert!(child.try_wait().unwrap().is_none());
    }
    std::fs::write(directory.join("descendant_ready"), b"ready").unwrap();
    terminal();
    // 故意不 wait 普通后代，验证父 owner 在 leader 已结束后仍检查自有组。
}

fn heartbeat_until_release(directory: &Path) {
    let started = Instant::now();
    let mut sequence = 0_u64;
    while !directory.join("release").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "fixture watchdog"
        );
        sequence += 1;
        let temporary = directory.join("heartbeat_next");
        std::fs::write(&temporary, sequence.to_string()).unwrap();
        std::fs::rename(temporary, directory.join("heartbeat")).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    // 此正向标记只可能由正常 release 分支写入，SIGKILL cleanup 无法生成。
    std::fs::write(directory.join("naturally_finished"), b"finished").unwrap();
}

fn wait_file(path: &Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "missing {path:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
