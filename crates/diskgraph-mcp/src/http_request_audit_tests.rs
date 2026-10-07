//! HTTP 请求日志固定实际认证主体的真实 socket 回归。
use crate::{McpConfig, McpService};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 保留原服务日志的测试见证，不改变实际分发和日志写入路径。
/// 来源：原生 Rust HTTP 请求审计回归，无 Java 对等对象。
struct CapturedLog(Arc<Mutex<Vec<u8>>>);
impl Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn socket_audit_keeps_admitted_subject_when_reauthentication_would_expire() {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut config = crate::auth::AuthConfig::single("fixture", "aud", b"audit-fixture");
    config.clock_skew_seconds = 0;
    let token = crate::auth::TokenMinter::new(b"audit-fixture").mint(&crate::auth::TokenClaims {
        issuer: "fixture".into(),
        audience: "aud".into(),
        subject: "reader".into(),
        expires_at_unix_seconds: now + 60,
        scope: Some("metadata:read".into()),
    });
    let expected = crate::auth::Authenticator::new(config.clone())
        .with_clock(move || now)
        .authenticate(Some(&token))
        .unwrap()
        .principal;
    service.engine().bootstrap_local_admin(&expected).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = Arc::clone(&calls);
    let auth = crate::auth::Authenticator::new(config).with_clock(move || {
        // 认证时有效；若响应后日志重认证，则该次时间已到token到期边界。
        if observed_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            now
        } else {
            now + 60
        }
    });
    let captured = Arc::new(Mutex::new(Vec::new()));
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = crate::http_server_runtime::HttpServerRuntime::start(
        service,
        listener,
        crate::http::ServerConfig::modern(
            crate::http::HttpLimits::default(),
            crate::http::Security::local(Some(&auth)),
        ),
        CapturedLog(Arc::clone(&captured)),
    )
    .unwrap();
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut reader = BufReader::new(stream);
    let response = read_response(&mut reader, 200);
    assert!(response["result"]["tools"].is_array());
    // 原连接再发未认证请求，确保每次入站都清空身份，不能继承上一请求主体。
    write!(
        reader.get_mut(),
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let denied = read_response(&mut reader, 401);
    assert!(denied.get("error").is_some());
    drop(reader);
    assert!(server.stop_and_join().is_ok_and(|result| result.is_ok()));
    let log = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    let requests: Vec<_> = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| entry["event"] == "request")
        .collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["principal"].as_str(), Some(expected.as_str()));
    assert_eq!(requests[1]["principal"], "anonymous");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "audit must reuse actual admitted identity"
    );
}

fn read_response(
    reader: &mut BufReader<std::net::TcpStream>,
    expected_status: u16,
) -> serde_json::Value {
    let length = read_head(reader, expected_status);
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).unwrap();
    serde_json::from_slice(&payload).unwrap()
}

fn read_head(reader: &mut BufReader<std::net::TcpStream>, expected_status: u16) -> usize {
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    let mut parts = status.split_whitespace();
    assert_eq!(parts.next(), Some("HTTP/1.1"));
    assert_eq!(
        parts.next().unwrap().parse::<u16>().unwrap(),
        expected_status
    );
    let mut length = None;
    let mut total = status.len();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        total += line.len();
        assert!(total < 16 * 1024 && !line.is_empty());
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let length = length.unwrap();
    assert!(length <= 4 * 1024 * 1024);
    length
}

#[test]
fn exhausted_generation_capture_does_not_renew_delivery_and_keeps_failed_audit_subject() {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let config = crate::auth::AuthConfig::single("fixture", "aud", b"deadline-audit-fixture");
    let token =
        crate::auth::TokenMinter::new(b"deadline-audit-fixture").mint(&crate::auth::TokenClaims {
            issuer: "fixture".into(),
            audience: "aud".into(),
            subject: "reader".into(),
            expires_at_unix_seconds: now + 60,
            scope: Some("metadata:read".into()),
        });
    let expected = crate::auth::Authenticator::new(config.clone())
        .with_clock(move || now)
        .authenticate(Some(&token))
        .unwrap()
        .principal;
    service.engine().bootstrap_local_admin(&expected).unwrap();
    let (entered, entry) = std::sync::mpsc::channel();
    let auth = crate::auth::Authenticator::new(config).with_clock(move || {
        entered.send(()).unwrap();
        now
    });
    let captured = Arc::new(Mutex::new(Vec::new()));
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let owner = service.engine().control_store().unwrap();
    let server = crate::http_server_runtime::HttpServerRuntime::start(
        service.clone(),
        listener,
        crate::http::ServerConfig::modern(
            crate::http::HttpLimits {
                read_timeout: Duration::from_millis(40),
                ..crate::http::HttpLimits::default()
            },
            crate::http::Security::local(Some(&auth)),
        ),
        CapturedLog(Arc::clone(&captured)),
    )
    .unwrap();
    let (finished, result) = std::sync::mpsc::channel();
    let client = std::thread::spawn(move || {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut bytes = Vec::new();
        let read = stream.read_to_end(&mut bytes);
        finished.send((read.is_ok(), bytes)).unwrap();
    });
    entry.recv_timeout(Duration::from_secs(2)).unwrap();
    // owner仍持锁时记录原连接已结束且没有发送；失败路径也释放锁并回收原线程。
    let bounded = result.recv_timeout(Duration::from_millis(500));
    drop(owner);
    client.join().unwrap();
    assert!(server.stop_and_join().is_ok_and(|result| result.is_ok()));
    let (closed, bytes) = bounded.expect("delivery waited for control owner release");
    assert!(
        closed && bytes.is_empty(),
        "deadline failure renewed error response delivery: {bytes:?}"
    );
    let log = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    let request = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|entry| entry["event"] == "request")
        .expect("failed delivery audit");
    assert_eq!(request["principal"].as_str(), Some(expected.as_str()));
    assert_eq!(request["delivery"], "failed");
}

#[cfg(any(unix, windows))]
fn actual_large_reply_interruption(case: &str) {
    #[cfg(unix)]
    use std::os::fd::AsRawFd;
    #[cfg(windows)]
    use std::os::windows::io::AsRawSocket;
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let expires = if case == "expiry" { now + 3 } else { u64::MAX };
    let auth = crate::auth::Authenticator::new(crate::auth::AuthConfig::single(
        "fixture",
        "aud",
        b"large-reply-fixture",
    ));
    let token =
        crate::auth::TokenMinter::new(b"large-reply-fixture").mint(&crate::auth::TokenClaims {
            issuer: "fixture".into(),
            audience: "aud".into(),
            subject: "reader".into(),
            expires_at_unix_seconds: expires,
            scope: Some("metadata:read".into()),
        });
    let identity = auth.authenticate(Some(&token)).unwrap();
    service
        .engine()
        .bootstrap_local_admin(&identity.principal)
        .unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let send_bytes: i32 = 4096;
    // 测试夹具限制监听socket继承的发送缓冲，使真实业务响应无法全部排入内核。
    // 原listener句柄仍存活，optval指向本地c_int，长度与其布局一致。
    #[cfg(unix)]
    let configured = unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&send_bytes as *const libc::c_int).cast(),
            std::mem::size_of_val(&send_bytes) as libc::socklen_t,
        )
    };
    #[cfg(windows)]
    let configured = unsafe {
        // 原 listener 由标准库建立 Winsock 生命周期；有效句柄和 i32 缓冲保持至调用结束。
        windows_sys::Win32::Networking::WinSock::setsockopt(
            listener.as_raw_socket() as usize,
            windows_sys::Win32::Networking::WinSock::SOL_SOCKET,
            windows_sys::Win32::Networking::WinSock::SO_SNDBUF,
            (&send_bytes as *const i32).cast(),
            std::mem::size_of_val(&send_bytes) as i32,
        )
    };
    assert_eq!(configured, 0, "native send buffer configuration failed");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let timeout = if matches!(case, "deadline" | "slow") {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(5)
    };
    let server = crate::http_server_runtime::HttpServerRuntime::start(
        service.clone(),
        listener,
        crate::http::ServerConfig::modern(
            crate::http::HttpLimits {
                max_body_bytes: 2 * 1024 * 1024,
                read_timeout: timeout,
                ..crate::http::HttpLimits::default()
            },
            crate::http::Security::local(Some(&auth)),
        ),
        CapturedLog(Arc::clone(&captured)),
    )
    .unwrap();
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(6)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let body =
        serde_json::json!({"jsonrpc":"2.0", "id":"x".repeat(1024 * 1024), "method":"tools/list"})
            .to_string();
    write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut reader = BufReader::new(stream);
    let length = read_head(&mut reader, 200);
    assert!(
        length > 512 * 1024,
        "fixture must pressure actual response delivery"
    );
    if case == "slow" {
        use std::sync::atomic::AtomicBool;
        let stop = Arc::new(AtomicBool::new(false));
        let peer_stop = Arc::clone(&stop);
        let peer = std::thread::spawn(move || {
            let mut received = Vec::new();
            let mut chunk = [0; 8192];
            while !peer_stop.load(Ordering::SeqCst) {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => received.extend_from_slice(&chunk[..count]),
                    Err(_) => break,
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            received
        });
        let observe_until = std::time::Instant::now() + Duration::from_secs(3);
        while server.active_connections() != 0 && std::time::Instant::now() < observe_until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let retired_before_stop = server.active_connections() == 0;
        stop.store(true, Ordering::SeqCst);
        let joined = server.stop_and_join().is_ok_and(|result| result.is_ok());
        let received = peer.join().unwrap();
        assert!(
            retired_before_stop,
            "slow read progress renewed real HTTP delivery"
        );
        assert!(joined && received.len() < length);
    } else {
        match case {
            "revoke" => service
                .engine()
                .control_store()
                .unwrap()
                .revoke_grant(
                    &identity.principal,
                    &diskgraph_core::Permission::MetadataRead,
                    &diskgraph_engine::admin_scope(),
                )
                .unwrap(),
            "expiry" => {
                while SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    < expires
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            "deadline" => {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                // 不读正文，等待原连接实际回收，不能以客户端主动断开伪造发送预算通过。
                while server.active_connections() != 0 && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert_eq!(
                    server.active_connections(),
                    0,
                    "nonreading peer renewed real HTTP delivery"
                );
            }
            _ => panic!("unknown fixture case"),
        }
        let mut received = Vec::new();
        let read = reader.read_to_end(&mut received);
        assert!(
            read.is_ok(),
            "actual interrupted reply did not close cleanly: {read:?}"
        );
        assert!(received.len() < length, "old result continued after {case}");
        drop(reader);
        assert!(server.stop_and_join().is_ok_and(|result| result.is_ok()));
    }
    let log = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
    let request = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|entry| entry["event"] == "request")
        .expect("actual interrupted delivery audit");
    assert_eq!(
        request["principal"].as_str(),
        Some(identity.principal.as_str())
    );
    assert_eq!(request["delivery"], "failed");
}

#[cfg(any(unix, windows))]
#[test]
fn actual_remote_nonreading_peer_cannot_renew_delivery_deadline() {
    actual_large_reply_interruption("deadline");
}
#[cfg(any(unix, windows))]
#[test]
fn actual_remote_revocation_stops_an_inflight_reply() {
    actual_large_reply_interruption("revoke");
}
#[cfg(any(unix, windows))]
#[test]
fn actual_remote_token_expiry_stops_an_inflight_reply() {
    actual_large_reply_interruption("expiry");
}

#[cfg(any(unix, windows))]
#[test]
fn actual_remote_slow_progress_cannot_renew_delivery_deadline() {
    actual_large_reply_interruption("slow");
}
