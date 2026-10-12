//! Linux 用户执行器出生失败必须发生在数据库创建之前。
#![cfg(target_os = "linux")]

use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[test]
fn incomplete_executor_birth_refuses_cli_without_creating_database() {
    for (birth, fd) in [
        (Some("{}"), None),
        (None, Some("5")),
        (Some("{}"), Some("4")),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path().join("unborn");
        let mut command = Command::new(env!("CARGO_BIN_EXE_diskgraph"));
        command.env_clear().args(["--json", "--data-dir"]);
        command.arg(&data).arg("doctor");
        if let Some(birth) = birth {
            command.env("DISKGRAPH_LINUX_EXECUTOR_BIRTH", birth);
        }
        if let Some(fd) = fd {
            command.env("DISKGRAPH_LINUX_EXECUTOR_FD", fd);
        }
        let result = bounded_output(command);
        assert!(!result.status.success(), "{result:?}");
        assert!(!data.exists(), "failed executor birth created database");
        let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"]["code"], "permission_denied");
    }
}

#[test]
fn mixed_legacy_and_executor_birth_refuses_cli_before_database() {
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("unborn");
    let mut command = Command::new(env!("CARGO_BIN_EXE_diskgraph"));
    command
        .env_clear()
        .env("DISKGRAPH_LINUX_EXECUTOR_BIRTH", "{}")
        .env("DISKGRAPH_LINUX_EXECUTOR_FD", "5")
        .env("DISKGRAPH_LINUX_SUPERVISOR_BIRTH", "{}")
        .args(["--json", "--data-dir"])
        .arg(&data)
        .arg("doctor");
    let result = bounded_output(command);
    assert!(!result.status.success(), "{result:?}");
    assert!(!data.exists());
    let reply: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reply["error"]["code"], "permission_denied");
}

// 观察预算只约束测试；超时杀死测试 child 不能算作产品已安全退休。
fn bounded_output(mut command: Command) -> Output {
    let mut stdout = tempfile::tempfile().unwrap();
    let mut stderr = tempfile::tempfile().unwrap();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.try_clone().unwrap()))
        .stderr(Stdio::from(stderr.try_clone().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut child = command.spawn().unwrap();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        let bytes = stdout.metadata().unwrap().len() + stderr.metadata().unwrap().len();
        if Instant::now() >= deadline || bytes > 128 << 10 {
            let _ = child.kill();
            let _ = child.wait();
            panic!("executor startup observation exceeded budget");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let read = |file: &mut std::fs::File| {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.take((128 << 10) + 1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len() <= 128 << 10);
        bytes
    };
    let stdout = read(&mut stdout);
    let stderr = read(&mut stderr);
    assert!(stdout.len() + stderr.len() <= 128 << 10);
    Output {
        status,
        stdout,
        stderr,
    }
}
