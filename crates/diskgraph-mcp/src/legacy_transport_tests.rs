use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::LegacyTransport;
use crate::McpService;
use crate::http::{HttpLimits, HttpRequest, Security};

fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

fn fixture() -> (tempfile::TempDir, Arc<McpService>) {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(crate::McpConfig {
        data_dir: dir.path().join("data"),
        profile: crate::protocol::ToolProfile::Manage,
        ..crate::McpConfig::default()
    })
    .unwrap();
    (dir, Arc::new(service))
}

#[test]
fn legacy_backpressure_refuses_before_enqueuing_an_index_job() {
    let (dir, service) = fixture();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let scope = service
        .engine()
        .register_scope(
            &root,
            service.context.principal(),
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    let transport = LegacyTransport::new(HttpLimits {
        max_response_bytes: 4096,
        ..HttpLimits::default()
    });
    let (id, receiver) = transport.registry.open(None).unwrap();
    let held: Vec<_> = (0..64)
        .map(|_| transport.registry.reserve(&id, None, 1).unwrap())
        .collect();
    let request = HttpRequest {
        method: "POST".into(), path: "/messages/".into(), query: format!("session_id={id}"), headers: HashMap::new(),
        body: json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"diskgraph_index","arguments":{"scope":scope.as_str()}}}).to_string(),
    };
    let log = Arc::new(Mutex::new(Vec::new()));
    let (client, mut server) = sockets();
    assert!(transport.post(
        &mut server,
        &request,
        &Security::local(None),
        &service,
        &log
    ));
    let mut reader = BufReader::new(client);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(status.contains("429"), "{status}");
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    let jobs = || {
        db.query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(jobs(), 0);
    drop(held);
    let (client, mut server) = sockets();
    assert!(transport.post(
        &mut server,
        &request,
        &Security::local(None),
        &service,
        &log
    ));
    let mut reader = BufReader::new(client);
    status.clear();
    reader.read_line(&mut status).unwrap();
    assert!(status.contains("202"), "{status}");
    let frame = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!frame.wire.contains("error"), "{}", frame.wire);
    assert_eq!(jobs(), 1);
}

#[test]
fn a_policy_change_prevents_old_results_from_starting_a_socket_write() {
    let (_dir, service) = fixture();
    let transport = LegacyTransport::new(HttpLimits::default());
    let version = LegacyTransport::authorization_generation(&service).unwrap();
    service
        .engine()
        .bootstrap_local_admin(&diskgraph_core::PrincipalId::new("other-fixture-user").unwrap())
        .unwrap();
    let (mut client, mut server) = sockets();
    server.set_nonblocking(true).unwrap();
    let error = transport
        .write_frame(
            &mut server,
            b"private result",
            &service,
            None,
            Some(version),
        )
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    server.shutdown(Shutdown::Write).unwrap();
    let mut data = Vec::new();
    client.read_to_end(&mut data).unwrap();
    assert!(data.is_empty());
}

#[test]
fn nonblocking_partial_writes_deliver_a_complete_large_frame() {
    let (_dir, service) = fixture();
    let transport = LegacyTransport::new(HttpLimits {
        read_timeout: Duration::from_secs(4),
        ..HttpLimits::default()
    });
    let (mut client, mut server) = sockets();
    server.set_nonblocking(true).unwrap();
    let wire = vec![b'x'; 4 << 20];
    let consumer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut received = Vec::new();
        client.read_to_end(&mut received).unwrap();
        received
    });
    transport
        .write_frame(
            &mut server,
            &wire,
            &service,
            None,
            Some(LegacyTransport::authorization_generation(&service).unwrap()),
        )
        .unwrap();
    server.shutdown(Shutdown::Write).unwrap();
    assert_eq!(consumer.join().unwrap(), wire);
}

#[test]
fn a_stalled_consumer_cannot_renew_the_frame_write_deadline() {
    let (_dir, service) = fixture();
    let transport = LegacyTransport::new(HttpLimits {
        read_timeout: Duration::from_millis(100),
        ..HttpLimits::default()
    });
    let (_client, mut server) = sockets();
    server.set_nonblocking(true).unwrap();
    let start = Instant::now();
    let error = transport
        .write_frame(&mut server, &vec![b'x'; 16 << 20], &service, None, None)
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn control_lock_contention_must_not_delay_a_frame_past_its_deadline() {
    let (_dir, service) = fixture();
    let transport = LegacyTransport::new(HttpLimits {
        read_timeout: Duration::from_millis(40),
        ..HttpLimits::default()
    });
    let generation = LegacyTransport::authorization_generation(&service).unwrap();
    let guard = service.engine().control_store().unwrap();
    let (mut client, mut server) = sockets();
    server.set_nonblocking(true).unwrap();
    let shared_service = Arc::clone(&service);
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = transport.write_frame(
            &mut server,
            b"private result",
            &shared_service,
            None,
            Some(generation),
        );
        server.shutdown(Shutdown::Write).unwrap();
        sender.send(result).unwrap();
    });
    let result = receiver.recv_timeout(Duration::from_millis(500));
    drop(guard);
    worker.join().unwrap();
    let result = result.expect("frame waited for the shared control lock instead of expiring");
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    let mut data = Vec::new();
    client.read_to_end(&mut data).unwrap();
    assert!(data.is_empty());
}

#[test]
fn a_sqlite_exclusive_lock_cannot_renew_the_frame_read_deadline() {
    let (dir, service) = fixture();
    let transport = LegacyTransport::new(HttpLimits {
        read_timeout: Duration::from_millis(40),
        ..HttpLimits::default()
    });
    let generation = LegacyTransport::authorization_generation(&service).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph-control.sqlite")).unwrap();
    let (sender, ready) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        db.execute_batch("BEGIN EXCLUSIVE;").unwrap();
        sender.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        db.execute_batch("ROLLBACK;").unwrap();
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    let (mut client, mut server) = sockets();
    server.set_nonblocking(true).unwrap();
    let result = transport.write_frame(
        &mut server,
        b"private result",
        &service,
        None,
        Some(generation),
    );
    server.shutdown(Shutdown::Write).unwrap();
    writer.join().unwrap();
    // 预算失败后回调与 busy_timeout 必须清理，下一次正常读仍可执行。
    assert_eq!(
        LegacyTransport::authorization_generation(&service),
        Some(generation)
    );
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    let mut data = Vec::new();
    client.read_to_end(&mut data).unwrap();
    assert!(data.is_empty());
}

#[test]
fn revoking_one_scope_stops_its_old_frame_while_the_subject_remains_live_elsewhere() {
    let (dir, service) = fixture();
    let scopes: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|name| {
            let root = dir.path().join(name);
            std::fs::create_dir(&root).unwrap();
            service
                .engine()
                .register_scope(
                    &root,
                    service.context.principal(),
                    &service.engine().policy_authorizer().unwrap(),
                )
                .unwrap()
        })
        .collect();
    let mut control = service.engine().control_store().unwrap();
    let epoch = control.policy_version().unwrap();
    // token 主体 ID 由签发者/subject 派生，使用实际认证身份配置 grant。
    let auth = crate::auth::Authenticator::new(crate::auth::AuthConfig::single(
        "fixture",
        "aud",
        b"isolated-grant-test-key",
    ));
    let token =
        crate::auth::TokenMinter::new(b"isolated-grant-test-key").mint(&crate::auth::TokenClaims {
            issuer: "fixture".into(),
            audience: "aud".into(),
            subject: "reader".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        });
    let identity = auth.authenticate(Some(&token)).unwrap();
    for scope in &scopes {
        control
            .upsert_grant(&diskgraph_core::Grant {
                principal: identity.principal.clone(),
                permission: diskgraph_core::Permission::MetadataRead,
                scope: scope.clone(),
                policy_version: epoch,
            })
            .unwrap();
    }
    let generation = control.authorization_generation().unwrap();
    control
        .revoke_grant(
            &identity.principal,
            &diskgraph_core::Permission::MetadataRead,
            &scopes[0],
        )
        .unwrap();
    assert_eq!(control.policy_version().unwrap(), epoch);
    drop(control);
    assert!(
        service.identity_is_live(&identity),
        "scope B remains granted"
    );
    let transport = LegacyTransport::new(HttpLimits::default());
    let (mut client, mut server) = sockets();
    server.set_nonblocking(true).unwrap();
    let result = transport.write_frame(
        &mut server,
        b"old scope A result",
        &service,
        Some(&identity),
        Some(generation),
    );
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    server.shutdown(Shutdown::Write).unwrap();
    let mut data = Vec::new();
    client.read_to_end(&mut data).unwrap();
    assert!(data.is_empty());
}
