//! HTTP 发送绝对期限回归，使用真实 TCP 连接。
use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let receiver = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (sender, _) = listener.accept().unwrap();
    sender
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    receiver
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    (sender, receiver)
}

#[test]
fn exhausted_deadline_never_sends_the_first_byte() {
    let (mut sender, mut receiver) = pair();
    let result = crate::http_delivery_writer::write_until(
        &mut sender,
        &[b"protected"],
        Instant::now(),
        || Ok(()),
    );
    drop(sender);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty(), "expired delivery emitted bytes");
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::TimedOut));
}

#[test]
fn authorization_time_consumes_the_same_delivery_deadline() {
    let (mut sender, mut receiver) = pair();
    let deadline = Instant::now() + Duration::from_millis(10);
    let result =
        crate::http_delivery_writer::write_until(&mut sender, &[b"protected"], deadline, || {
            std::thread::sleep(Duration::from_millis(30));
            Ok(())
        });
    drop(sender);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty(), "authorization renewed expired delivery");
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::TimedOut));
}

#[test]
fn a_nonreading_peer_cannot_extend_the_absolute_deadline() {
    let (mut sender, _receiver) = pair();
    let payload = vec![b'x'; 8 * 1024 * 1024];
    let start = Instant::now();
    let result = crate::http_delivery_writer::write_until(
        &mut sender,
        &[&payload],
        start + Duration::from_millis(80),
        || Ok(()),
    );
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::TimedOut));
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "socket timeout renewed the delivery window"
    );
}

fn service() -> (tempfile::TempDir, crate::McpService) {
    let dir = tempfile::tempdir().unwrap();
    let service = crate::McpService::open(crate::McpConfig {
        data_dir: dir.path().join("data"),
        ..crate::McpConfig::default()
    })
    .unwrap();
    (dir, service)
}

#[test]
fn changed_authorization_prevents_the_first_byte() {
    let (_dir, service) = service();
    let deadline = Instant::now() + Duration::from_secs(2);
    let version = crate::http_delivery_authority::generation_until(&service, deadline).unwrap();
    service
        .engine()
        .bootstrap_local_admin(&diskgraph_core::PrincipalId::new("changed-policy").unwrap())
        .unwrap();
    let (mut sender, mut receiver) = pair();
    let result = crate::http_delivery_writer::write_until(
        &mut sender,
        &[b"private result"],
        deadline,
        || {
            crate::http_delivery_authority::authorize_until(
                &service,
                &service.context,
                version,
                deadline,
            )
        },
    );
    drop(sender);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::PermissionDenied));
}

#[test]
fn expired_request_context_prevents_the_first_byte() {
    let (_dir, service) = service();
    let deadline = Instant::now() + Duration::from_secs(2);
    let version = crate::http_delivery_authority::generation_until(&service, deadline).unwrap();
    let identity = crate::auth::AuthenticatedPrincipal {
        principal: service.context.principal().clone(),
        issuer: "fixture".into(),
        permissions: vec![diskgraph_core::Permission::MetadataRead],
        expires_at_unix_seconds: 0,
    };
    let context = crate::request_context::RequestContext::authenticated(&identity, "http");
    let (mut sender, mut receiver) = pair();
    let result = crate::http_delivery_writer::write_until(
        &mut sender,
        &[b"private result"],
        deadline,
        || crate::http_delivery_authority::authorize_until(&service, &context, version, deadline),
    );
    drop(sender);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::PermissionDenied));
}

#[test]
fn authorization_change_between_chunks_stops_remaining_bytes() {
    let (_dir, service) = service();
    let deadline = Instant::now() + Duration::from_secs(2);
    let version = crate::http_delivery_authority::generation_until(&service, deadline).unwrap();
    let (mut sender, mut receiver) = pair();
    let payload = vec![b'x'; 128 * 1024];
    let mut checks = 0;
    let result =
        crate::http_delivery_writer::write_until(&mut sender, &[&payload], deadline, || {
            checks += 1;
            if checks == 2 {
                service
                    .engine()
                    .bootstrap_local_admin(
                        &diskgraph_core::PrincipalId::new("changed-between-chunks").unwrap(),
                    )
                    .unwrap();
            }
            crate::http_delivery_authority::authorize_until(
                &service,
                &service.context,
                version,
                deadline,
            )
        });
    drop(sender);
    let mut bytes = Vec::new();
    receiver.read_to_end(&mut bytes).unwrap();
    assert!(!bytes.is_empty() && bytes.len() <= 64 * 1024);
    assert_eq!(checks, 2);
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::PermissionDenied));
}

fn session_for_test_id(id: &str) -> String {
    let (_dir, mut service) = service();
    let request = crate::http::HttpRequest {
        method: "POST".into(),
        path: "/mcp".into(),
        query: String::new(),
        headers: Default::default(),
        body: serde_json::json!({"jsonrpc":"2.0", "id":id, "method":"tools/list"}).to_string(),
    };
    crate::http::handle(&mut service, &request, &crate::http::HttpLimits::default())
        .session
        .unwrap()
}

#[test]
fn long_request_id_cannot_expand_the_http_session_header() {
    assert!(session_for_test_id(&"x".repeat(1024)).len() <= 128);
}
#[test]
fn unicode_request_id_keeps_the_session_header_printable_ascii() {
    let session = session_for_test_id("中文");
    assert!(session.bytes().all(|byte| (b'!'..=b'~').contains(&byte)));
}

#[test]
fn successful_delivery_restores_blocking_reads_for_the_next_request() {
    use std::io::Write;
    let (mut sender, mut receiver) = pair();
    sender
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    crate::http_delivery_writer::write_until(
        &mut sender,
        &[b"ok"],
        Instant::now() + Duration::from_secs(1),
        || Ok(()),
    )
    .unwrap();
    let peer = std::thread::spawn(move || {
        let mut response = [0; 2];
        receiver.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"ok");
        std::thread::sleep(Duration::from_millis(30));
        receiver.write_all(b"next").unwrap();
    });
    let mut next = [0; 4];
    let read = sender.read_exact(&mut next);
    peer.join().unwrap();
    read.expect("delivery left the original socket nonblocking");
    assert_eq!(&next, b"next");
}
