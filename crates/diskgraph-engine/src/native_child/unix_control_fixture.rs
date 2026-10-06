//! 独立 Unix stdin fixture；来源：Rust 测试可执行文件与真实 socket/进程语义。

use super::ControlWriteStatus;
use super::unix_control_test_support::{command, complete, spawn, wait_file};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

#[test]
fn stdin_fixture() {
    let Ok(mode) = std::env::var("DG_CONTROL_FIXTURE") else {
        return;
    };
    let directory = std::path::PathBuf::from(std::env::var_os("DG_CONTROL_DIRECTORY").unwrap());
    std::fs::write(directory.join("pid"), std::process::id().to_string()).unwrap();
    match mode.as_str() {
        "echo" => echo(&directory),
        "blocked" => blocked(&directory),
        "peer_closed" => peer_closed(&directory),
        "linger" => {
            std::fs::write(directory.join("ready"), b"ready").unwrap();
            release(&directory);
        }
        "sigpipe_host" => sigpipe_host(&directory),
        other => panic!("unknown isolated fixture mode {other}"),
    }
    // 普通退出使外层测试无需把测试 harness 的结束文本当作应用协议。
    std::process::exit(0);
}

fn echo(directory: &Path) {
    let mut bytes = Vec::new();
    std::io::stdin().take(8193).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 8192);
    std::fs::write(directory.join("received"), &bytes).unwrap();
    println!("DG_CONTROL_HEX={}", hex::encode(bytes));
    println!("DG_CONTROL_EOF=true");
}

fn blocked(directory: &Path) {
    std::fs::write(directory.join("ready"), b"ready").unwrap();
    release(directory);
    let mut buffer = [0; 4096];
    let mut hasher = Sha256::new();
    let mut count = 0_u64;
    loop {
        let read = std::io::stdin().read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        count += read as u64;
        assert!(count <= 32 * 1024 * 1024, "fixture input was not bounded");
        hasher.update(&buffer[..read]);
    }
    println!("DG_CONTROL_COUNT={count}");
    println!("DG_CONTROL_SHA256={}", hex::encode(hasher.finalize()));
    println!("DG_CONTROL_EOF=true");
}

fn peer_closed(directory: &Path) {
    // 安全性：仅独立 fixture 关闭自身 stdin；不改变父宿主的标准输入。
    assert_eq!(unsafe { libc::close(0) }, 0);
    std::fs::write(directory.join("closed"), b"closed").unwrap();
    release(directory);
}

fn sigpipe_host(directory: &Path) {
    // 安全性：恢复 SIGPIPE 默认行为仅发生在隔离宿主进程，生产代码不得这么做。
    assert_ne!(
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) },
        libc::SIG_ERR
    );
    let leaf = directory.join("leaf");
    std::fs::create_dir(&leaf).unwrap();
    let mut child = spawn("peer_closed", &leaf);
    wait_file(&leaf.join("closed"));
    assert!(
        !child.poll().unwrap(),
        "peer must remain alive after closing stdin"
    );
    let error = child.start_control_write(&[0, 128, 255]).unwrap_err();
    let original = error.native_io_error().expect("original EPIPE I/O");
    assert_eq!(original.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(original.raw_os_error(), Some(libc::EPIPE));
    assert_eq!(
        child.request_control_close().unwrap(),
        ControlWriteStatus::Closed
    );
    std::fs::write(leaf.join("release"), b"release").unwrap();
    complete(&mut child, &leaf);
    println!("DG_SIGPIPE_SAFE=true");
    std::io::stdout().flush().unwrap();
}

fn release(directory: &Path) {
    let started = Instant::now();
    while !directory.join("release").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "fixture watchdog"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// 构造只在隔离进程恢复默认 SIGPIPE 的宿主。参数：directory 为隔离目录；返回：当前测试二进制命令。
pub(super) fn isolated_host(directory: &Path) -> std::process::Command {
    command("sigpipe_host", directory)
}
