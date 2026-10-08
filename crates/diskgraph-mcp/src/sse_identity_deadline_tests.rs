//! SSE身份观察与真实控制owner竞争回归。
use crate::{McpConfig, McpService};
use std::sync::{Arc, mpsc};
use std::time::Duration;
#[test]
fn identity_check_refuses_while_control_owner_still_holds_lock() {
    let dir = tempfile::tempdir().unwrap();
    let service = Arc::new(
        McpService::open(McpConfig {
            data_dir: dir.path().join("data"),
            ..McpConfig::default()
        })
        .unwrap(),
    );
    let auth = crate::auth::Authenticator::new(crate::auth::AuthConfig::single(
        "fixture",
        "aud",
        b"sse-lock-fixture",
    ));
    let token =
        crate::auth::TokenMinter::new(b"sse-lock-fixture").mint(&crate::auth::TokenClaims {
            issuer: "fixture".into(),
            audience: "aud".into(),
            subject: "reader".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        });
    let identity = auth.authenticate(Some(&token)).unwrap();
    let owner = service.engine().control_store().unwrap();
    let (entered, entry) = mpsc::channel();
    let (finished, result) = mpsc::channel();
    let request = Arc::clone(&service);
    let worker = std::thread::spawn(move || {
        entered.send(()).unwrap();
        let live = request.identity_is_live(&identity);
        finished.send(live).unwrap();
    });
    entry.recv_timeout(Duration::from_secs(2)).unwrap();
    // 先记录owner仍持锁时的结果，再释放并join，失败测试也不遗留阻塞线程。
    let bounded = result.recv_timeout(Duration::from_millis(250));
    drop(owner);
    worker.join().unwrap();
    assert!(
        matches!(bounded, Ok(false)),
        "identity check waited for owner release: {bounded:?}"
    );
}

fn connect_stream(port: u16, path: &str, token: &str) -> std::net::TcpStream {
    use std::io::Write;
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\n"
    )
    .unwrap();
    expect_stream_handshake(&mut stream);
    stream
}

fn expect_stream_handshake(stream: &mut std::net::TcpStream) {
    use std::io::Read;
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
        assert!(head.len() < 16 * 1024);
    }
    assert!(head.starts_with(b"HTTP/1.1 200"), "{head:?}");
}

fn actual_stream_closes_under_control_contention(path: &str) {
    use std::io::Read;
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let auth = crate::auth::Authenticator::new(crate::auth::AuthConfig::single(
        "fixture",
        "aud",
        b"socket-lock-fixture",
    ));
    let token =
        crate::auth::TokenMinter::new(b"socket-lock-fixture").mint(&crate::auth::TokenClaims {
            issuer: "fixture".into(),
            audience: "aud".into(),
            subject: "reader".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        });
    let identity = auth.authenticate(Some(&token)).unwrap();
    service
        .engine()
        .bootstrap_local_admin(&identity.principal)
        .unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = crate::http_server_runtime::HttpServerRuntime::start(
        service.clone(),
        listener,
        crate::http::ServerConfig::modern(
            crate::http::HttpLimits {
                max_requests_per_second_per_client: 100,
                ..crate::http::HttpLimits::default()
            },
            crate::http::Security::local(Some(&auth)),
        )
        .with_legacy(),
        Vec::new(),
    )
    .unwrap();
    // 四条真实连接占满主体槽；控制 owner 保持持锁直到记录关闭结果。
    let mut streams: Vec<_> = (0..4).map(|_| connect_stream(port, path, &token)).collect();
    let owner = service.engine().control_store().unwrap();
    let mut closed = Vec::new();
    for stream in &mut streams {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut bytes = [0; 4096];
        let eof = loop {
            if Instant::now() >= deadline {
                break false;
            }
            match stream.read(&mut bytes) {
                Ok(0) => break true,
                Ok(_) => {}
                Err(_) => break false,
            }
        };
        closed.push(eof);
    }
    let release_deadline = Instant::now() + Duration::from_secs(2);
    while server.active_connections() != 0 && Instant::now() < release_deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let active = server.active_connections();
    drop(owner);
    assert!(
        closed.iter().all(|closed| *closed),
        "stream EOF while owner held: {closed:?}"
    );
    assert_eq!(
        active, 0,
        "original connections must release before owner unlock"
    );
    // 在同一个服务 owner 上重开四个槽，不能用新服务的空配额替代释放证据。
    let reopened: Vec<_> = (0..4).map(|_| connect_stream(port, path, &token)).collect();
    let owner = service.engine().control_store().unwrap();
    let (finished, result) = mpsc::channel();
    let joiner = std::thread::spawn(move || {
        finished
            .send(server.stop_and_join().is_ok_and(|outcome| outcome.is_ok()))
            .unwrap();
    });
    let joined = result.recv_timeout(Duration::from_secs(2));
    // 失败也先释放 owner 再回收线程，避免回归测试将整个测试进程挂住。
    drop(owner);
    joiner.join().unwrap();
    drop(reopened);
    assert!(
        matches!(joined, Ok(true)),
        "join waited for owner unlock: {joined:?}"
    );
}

#[test]
fn modern_socket_closes_and_joins_while_control_owner_holds_lock() {
    actual_stream_closes_under_control_contention("/mcp");
}
#[test]
fn legacy_socket_closes_and_joins_while_control_owner_holds_lock() {
    actual_stream_closes_under_control_contention("/sse");
}

fn assert_modern_authenticated_stream_rejects_client_noise(buffered: bool) {
    use std::io::{Read, Write};
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let auth = crate::auth::Authenticator::new(crate::auth::AuthConfig::single(
        "fixture",
        "aud",
        b"noise-fixture",
    ));
    let token = crate::auth::TokenMinter::new(b"noise-fixture").mint(&crate::auth::TokenClaims {
        issuer: "fixture".into(),
        audience: "aud".into(),
        subject: "reader".into(),
        expires_at_unix_seconds: u64::MAX,
        scope: Some("metadata:read".into()),
    });
    service
        .engine()
        .bootstrap_local_admin(&auth.authenticate(Some(&token)).unwrap().principal)
        .unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = crate::http_server_runtime::HttpServerRuntime::start(
        service,
        listener,
        crate::http::ServerConfig::modern(
            crate::http::HttpLimits::default(),
            crate::http::Security::local(Some(&auth)),
        ),
        Vec::new(),
    )
    .unwrap();
    let mut stream = if buffered {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        // 单次写入包含完整GET头和噪音，覆盖请求reader预读而socket peek不可见的输入。
        write!(stream, "GET /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\nunexpected-client-byte").unwrap();
        expect_stream_handshake(&mut stream);
        stream
    } else {
        let mut stream = connect_stream(port, "/mcp", &token);
        stream.write_all(b"unexpected-client-byte").unwrap();
        stream
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = [0; 4096];
    let closed = loop {
        if Instant::now() >= deadline {
            break false;
        }
        match stream.read(&mut bytes) {
            Ok(0) => break true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break true,
            Err(_) => break false,
        }
    };
    drop(stream);
    assert!(server.stop_and_join().is_ok_and(|result| result.is_ok()));
    assert!(
        closed,
        "server kept authenticated GET stream alive after client noise"
    );
}

#[test]
fn modern_authenticated_stream_rejects_client_noise() {
    assert_modern_authenticated_stream_rejects_client_noise(false);
}
#[test]
fn modern_authenticated_stream_rejects_noise_prefetched_with_get_headers() {
    assert_modern_authenticated_stream_rejects_client_noise(true);
}
