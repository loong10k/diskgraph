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
    assert!(length < 4 * 1024 * 1024);
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload).unwrap();
    serde_json::from_slice(&payload).unwrap()
}
