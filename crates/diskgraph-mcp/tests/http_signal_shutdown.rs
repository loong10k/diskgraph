//! Unix 产品 HTTP 服务的真实信号退出，不把进程被杀计为正常回收。
#![cfg(unix)]
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
/// 测试持有原 child，异常时实际 kill/wait 后才删除隔离目录。
struct OriginalChild(Child);
impl Drop for OriginalChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn expect_normal_signal_shutdown(signal: libc::c_int) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key");
    std::fs::write(&key, b"signal-qualification-key-at-least-32-bytes").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let log = std::fs::File::create(dir.path().join("stderr")).unwrap();
    let mut child = OriginalChild(
        Command::new(env!("CARGO_BIN_EXE_diskgraph-mcp"))
            .args([
                "--transport",
                "streamable-http",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--auth-key-file",
                "signal-test",
                "signal-client",
            ])
            .arg(&key)
            .arg("--data-dir")
            .arg(dir.path().join("data"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "server exited before readiness"
        );
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(Instant::now() < until, "server did not listen");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        unsafe { libc::kill(child.0.id() as libc::pid_t, signal) },
        0
    );
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < until, "original server did not stop");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "signal killed the original process instead of completing normal retirement: {status}"
    );
}

#[test]
fn sigterm_stops_actual_http_runtime_and_returns_success() {
    expect_normal_signal_shutdown(libc::SIGTERM);
}
#[test]
fn sigint_stops_actual_http_runtime_and_returns_success() {
    expect_normal_signal_shutdown(libc::SIGINT);
}
