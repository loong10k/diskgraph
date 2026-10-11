//! Linux MCP真实二进制必须在数据库前拒绝不完整或伪造的监督出生配置。
#![cfg(target_os = "linux")]
use std::process::{Command, Stdio};

#[test]
fn invalid_supervisor_birth_never_falls_back_to_local_bootstrap() {
    for (birth, fd) in [
        (Some("{}"), None),
        (None, Some("0")),
        (Some("{}"), Some("0")),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("not_born");
        let mut command = Command::new(env!("CARGO_BIN_EXE_diskgraph-mcp"));
        command
            .env_clear()
            .args(["--transport", "stdio", "--data-dir"])
            .arg(&data)
            .stdin(Stdio::null());
        if let Some(birth) = birth {
            command.env("DISKGRAPH_LINUX_SUPERVISOR_BIRTH", birth);
        }
        if let Some(fd) = fd {
            command.env("DISKGRAPH_LINUX_SUPERVISOR_FD", fd);
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(6),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(
            !data.exists(),
            "invalid birth silently enabled ordinary local service"
        );
    }
}
