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
fn expect_normal_signal_shutdown(signal: libc::c_int, debug_unicode: bool) {
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
            .env_remove("DISKGRAPH_HTTP_DEBUG")
            .envs(debug_unicode.then_some(("DISKGRAPH_HTTP_DEBUG", "1")))
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
        // 空闲端口释放后可能被另一夹具复用；必须先证明本 child 已绑定，不能仅探测任意 listener。
        let own_listener = std::fs::read_to_string(dir.path().join("stderr"))
            .unwrap_or_default()
            .contains(&format!(
                "diskgraph-mcp listening on http://127.0.0.1:{port}/mcp"
            ));
        if own_listener && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        assert!(Instant::now() < until, "server did not listen");
        std::thread::sleep(Duration::from_millis(10));
    }
    if debug_unicode {
        use std::io::{Read, Write};
        let exchange = |request: String| {
            let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket.write_all(request.as_bytes()).unwrap();
            let mut header = Vec::new();
            let mut byte = [0];
            while !header.ends_with(b"\r\n\r\n") {
                assert!(
                    header.len() < 16 << 10 && Instant::now() < until,
                    "response header budget"
                );
                socket.read_exact(&mut byte).unwrap_or_else(|error| {
                    let stderr = std::fs::read_to_string(dir.path().join("stderr"))
                        .unwrap_or_else(|read_error| format!("stderr unavailable: {read_error}"));
                    panic!("original HTTP response failed: {error}; server stderr: {stderr}");
                });
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap();
            let length: usize = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse().unwrap())
                })
                .expect("bounded HTTP response must provide length");
            assert!(length <= 64 << 10, "response body budget");
            let mut body = vec![0; length];
            socket.read_exact(&mut body).unwrap();
            format!("{header}{}", String::from_utf8(body).unwrap())
        };
        let body = format!("{}中", "a".repeat(119));
        let refused = exchange(format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
        let health = exchange(
            "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".into(),
        );
        assert!(health.starts_with("HTTP/1.1 200"), "{health}");
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "debug request stopped the original service"
        );
        let stderr = std::fs::read_to_string(dir.path().join("stderr")).unwrap();
        assert!(
            stderr.contains("body_bytes=122"),
            "debug logging was not exercised"
        );
        assert!(
            !stderr.contains(&body),
            "debug disclosed the raw request body"
        );
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
    expect_normal_signal_shutdown(libc::SIGTERM, false);
}
#[test]
fn sigint_stops_actual_http_runtime_and_returns_success() {
    expect_normal_signal_shutdown(libc::SIGINT, false);
}

#[test]
fn debug_unicode_request_keeps_the_original_authenticated_listener_alive() {
    expect_normal_signal_shutdown(libc::SIGTERM, true);
}
