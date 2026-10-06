#[path = "support/native_scan_service.rs"]
mod native_scan_service;
use diskgraph_mcp::McpConfig;
use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits, HttpRequest};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn a_valid_unscoped_token_does_not_inherit_the_local_admin() {
    let dir = tempfile::tempdir().unwrap();
    let (mut service, _scan_recovery) = native_scan_service::open(
        McpConfig {
            data_dir: dir.path().join("data"),
            profile: diskgraph_mcp::protocol::ToolProfile::Manage,
            ..McpConfig::default()
        },
        false,
    )
    .unwrap();
    let auth = Authenticator::new(AuthConfig::single("issuer", "aud", b"test-key"));
    let token = TokenMinter::new(b"test-key").mint(&TokenClaims {
        issuer: "issuer".into(),
        audience: "aud".into(),
        subject: "stranger".into(),
        expires_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300,
        scope: None,
    });
    let request = HttpRequest { method: "POST".into(), path: "/mcp".into(), query: String::new(), headers: HashMap::from([("authorization".into(), format!("Bearer {token}"))]), body: r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"diskgraph_scope","arguments":{"action":"list"}}}"#.into() };
    let response =
        http::handle_authenticated(&mut service, &request, &HttpLimits::default(), Some(&auth));
    let value: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(
        value["error"]["data"]["business_code"], "permission_denied",
        "{value}"
    );
}

#[test]
fn a_real_sse_connection_cannot_bypass_origin_or_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _scan_recovery) = native_scan_service::open(
        McpConfig {
            data_dir: dir.path().join("data"),
            profile: diskgraph_mcp::protocol::ToolProfile::Manage,
            ..McpConfig::default()
        },
        false,
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let auth = Authenticator::new(AuthConfig::single("issuer", "aud", b"test-key"));
    std::thread::spawn(move || {
        let _ = http::serve_authenticated(
            service,
            listener,
            &HttpLimits::default(),
            Some(&auth),
            std::io::sink(),
        );
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .write_all(
            b"GET /mcp HTTP/1.1\r\nHost: localhost\r\nOrigin: https://hostile.example\r\n\r\n",
        )
        .unwrap();
    let mut buffer = [0_u8; 2048];
    let count = stream.read(&mut buffer).unwrap();
    let response = String::from_utf8_lossy(&buffer[..count]);
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[test]
fn node_and_top_do_not_decode_unrelated_revision_rows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let mut state = 0x1234_5678_u32;
    let large: Vec<u8> = (0..65536)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    std::fs::write(root.join("middle"), &large[..8192]).unwrap();
    std::fs::write(root.join("largest"), large).unwrap();
    std::fs::write(root.join("unrelated"), [0]).unwrap();
    let (mut service, _scan_recovery) = native_scan_service::open(
        McpConfig {
            data_dir: dir.path().join("data"),
            ..McpConfig::default()
        },
        false,
    )
    .unwrap();
    let principal = diskgraph_core::PrincipalId::new(diskgraph_mcp::STDIO_PRINCIPAL).unwrap();
    let scope = service
        .engine()
        .register_scope(
            &root,
            &principal,
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    let job = service
        .engine()
        .index_scope(
            &scope,
            &principal,
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    service.engine().run_job(&job.job_id, "fixture").unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/diskgraph.sqlite")).unwrap();
    db.execute(
        "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'unrelated'",
        [],
    )
    .unwrap();
    for tool in ["diskgraph_node", "diskgraph_top", "diskgraph_search"] {
        let mut arguments = serde_json::json!({"scope":scope.as_str()});
        if tool != "diskgraph_node" {
            arguments["limit"] = serde_json::json!(1);
        }
        if tool == "diskgraph_search" {
            arguments["pattern"] = serde_json::json!("largest");
        }
        let frame = diskgraph_mcp::protocol::decode_request(&serde_json::json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":tool, "arguments":arguments}}).to_string()).unwrap();
        let response = service.handle(&frame);
        assert_eq!(response["result"]["isError"], false, "{tool}: {response}");
    }
    // The one-row lookahead may decode row 2, but not unrelated row 3.
    let frame = diskgraph_mcp::protocol::decode_request(&serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"diskgraph_top","arguments":{"scope":scope.as_str(),"limit":2}}}).to_string()).unwrap();
    assert!(service.handle(&frame).get("error").is_some());
}

#[test]
fn negative_token_expiry_is_malformed() {
    let claims = serde_json::json!({"iss":"issuer", "aud":"aud", "sub":"subject", "exp":-1});
    assert!(TokenClaims::from_payload(&claims).is_none());
}

#[test]
fn legacy_sessions_are_bound_to_the_handshake_principal() {
    let registry = diskgraph_mcp::legacy::SessionRegistry::new();
    let (session, _receiver) = registry.open_bound(Some("alice".into()));
    assert!(registry.lookup_bound(&session, Some("alice")).is_some());
    assert!(registry.lookup_bound(&session, Some("bob")).is_none());
    assert!(registry.lookup_bound(&session, None).is_none());
}

#[test]
fn issuers_cannot_alias_the_same_subject() {
    use diskgraph_mcp::auth::IssuerConfig;
    let mut config = AuthConfig::single("first", "aud", b"test-key");
    config.issuers.push(IssuerConfig {
        issuer: "second".into(),
        audience: "aud".into(),
        verification_key: b"test-key".to_vec(),
    });
    let auth = Authenticator::new(config);
    let identities = ["first", "second"].map(|issuer| {
        let token = TokenMinter::new(b"test-key").mint(&TokenClaims {
            issuer: issuer.into(),
            audience: "aud".into(),
            subject: "same-subject".into(),
            expires_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 300,
            scope: Some("metadata:read".into()),
        });
        auth.authenticate(Some(&token)).unwrap().principal
    });
    assert_ne!(identities[0], identities[1]);
}

#[test]
fn socket_sse_quota_is_per_subject_and_revocation_closes_connections() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _scan_recovery) = native_scan_service::open(
        McpConfig {
            data_dir: dir.path().join("data"),
            ..McpConfig::default()
        },
        true,
    )
    .unwrap();
    let control_path = dir.path().join("data/diskgraph-control.sqlite");
    let auth = Authenticator::new(AuthConfig::single("issuer", "aud", b"test-key"));
    let tokens = ["alice", "bob"].map(|subject| {
        TokenMinter::new(b"test-key").mint(&TokenClaims {
            issuer: "issuer".into(),
            audience: "aud".into(),
            subject: subject.into(),
            expires_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 300,
            scope: Some("scope:admin".into()),
        })
    });
    for token in &tokens {
        service
            .engine()
            .bootstrap_local_admin(&auth.authenticate(Some(token)).unwrap().principal)
            .unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = http::serve_authenticated(
            service,
            listener,
            &HttpLimits::default(),
            Some(&auth),
            std::io::sink(),
        );
    });
    let connect = |token: &str| {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .write_all(
                format!(
                    "GET /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();
        let mut bytes = Vec::new();
        let mut byte = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            bytes.push(byte[0]);
        }
        (stream, String::from_utf8(bytes).unwrap())
    };
    let mut held = Vec::new();
    for _ in 0..4 {
        let (stream, header) = connect(&tokens[0]);
        assert!(header.starts_with("HTTP/1.1 200"), "{header}");
        held.push(stream);
    }
    let (_rejected, header) = connect(&tokens[0]);
    assert!(header.starts_with("HTTP/1.1 429"), "{header}");
    let (bob, header) = connect(&tokens[1]);
    assert!(header.starts_with("HTTP/1.1 200"), "{header}");
    held.push(bob);
    diskgraph_store::ControlStore::open(&control_path)
        .unwrap()
        .revoke_policy()
        .unwrap();
    for mut stream in held {
        let started = std::time::Instant::now();
        let mut bytes = [0; 1024];
        loop {
            assert!(started.elapsed() < Duration::from_secs(5));
            match stream.read(&mut bytes) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(error) => panic!("revoked SSE did not close: {error}"),
            }
        }
    }
}

#[test]
fn socket_sse_expires_with_its_authenticated_token() {
    let dir = tempfile::tempdir().unwrap();
    let (service, _scan_recovery) = native_scan_service::open(
        McpConfig {
            data_dir: dir.path().join("data"),
            ..McpConfig::default()
        },
        true,
    )
    .unwrap();
    let auth = Authenticator::new(AuthConfig::single("issuer", "aud", b"test-key"));
    let token = TokenMinter::new(b"test-key").mint(&TokenClaims {
        issuer: "issuer".into(),
        audience: "aud".into(),
        subject: "expiring".into(),
        expires_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3,
        scope: Some("scope:admin".into()),
    });
    service
        .engine()
        .bootstrap_local_admin(&auth.authenticate(Some(&token)).unwrap().principal)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = http::serve_authenticated(
            service,
            listener,
            &HttpLimits::default(),
            Some(&auth),
            std::io::sink(),
        );
    });
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /mcp HTTP/1.1\r\nAuthorization: Bearer {token}\r\nHost: localhost\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let started = std::time::Instant::now();
    let mut bytes = [0; 2048];
    let mut header = Vec::new();
    loop {
        assert!(started.elapsed() < Duration::from_secs(6));
        match stream.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => header.extend_from_slice(&bytes[..count]),
            Err(error) => panic!("expired stream did not close: {error}"),
        }
    }
    assert!(String::from_utf8_lossy(&header).starts_with("HTTP/1.1 200"));
}

#[test]
fn socket_requests_intersect_subject_grants_and_token_capabilities() {
    use diskgraph_core::{Grant, Permission, PrincipalId};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("file"), [0]).unwrap();
    let (service, _scan_recovery) = native_scan_service::open(
        McpConfig {
            data_dir: dir.path().join("data"),
            ..Default::default()
        },
        true,
    )
    .unwrap();
    let local = PrincipalId::new("fixture-admin").unwrap();
    service.engine().bootstrap_local_admin(&local).unwrap();
    let scope = service
        .engine()
        .register_scope(
            &root,
            &local,
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    let job = service
        .engine()
        .index_scope(
            &scope,
            &local,
            &service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    service.engine().run_job(&job.job_id, "fixture").unwrap();
    let auth = Authenticator::new(AuthConfig::single("issuer", "aud", b"test-key"));
    let tokens = [
        ("alice", Some("metadata:read")),
        ("bob", Some("metadata:read")),
        ("alice", None),
    ]
    .map(|(subject, capabilities)| {
        TokenMinter::new(b"test-key").mint(&TokenClaims {
            issuer: "issuer".into(),
            audience: "aud".into(),
            subject: subject.into(),
            expires_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 300,
            scope: capabilities.map(str::to_owned),
        })
    });
    let alice = auth.authenticate(Some(&tokens[0])).unwrap().principal;
    let control_path = dir.path().join("data/diskgraph-control.sqlite");
    let mut control = diskgraph_store::ControlStore::open(&control_path).unwrap();
    control
        .upsert_grant(&Grant {
            principal: alice.clone(),
            permission: Permission::MetadataRead,
            scope: scope.clone(),
            policy_version: control.policy_version().unwrap(),
        })
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = http::serve_authenticated(
            service,
            listener,
            &HttpLimits::default(),
            Some(&auth),
            std::io::sink(),
        );
    });
    let call = |token: &str| {
        let body=serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"diskgraph_node","arguments":{"scope":scope.as_str()}}}).to_string();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(format!("POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
        let mut header = Vec::new();
        let mut byte = [0];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
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
            .unwrap();
        let mut response = vec![0; length];
        stream.read_exact(&mut response).unwrap();
        serde_json::from_slice::<serde_json::Value>(&response).unwrap()
    };
    assert_eq!(call(&tokens[0])["result"]["isError"], false);
    for token in &tokens[1..] {
        assert_eq!(
            call(token)["error"]["data"]["business_code"],
            "permission_denied"
        );
    }
    control
        .revoke_grant(&alice, &Permission::MetadataRead, &scope)
        .unwrap();
    assert_eq!(
        call(&tokens[0])["error"]["data"]["business_code"],
        "permission_denied"
    );
}

#[test]
fn opening_remote_on_a_local_database_does_not_reuse_local_privileges() {
    let dir = tempfile::tempdir().unwrap();
    let config = McpConfig {
        data_dir: dir.path().join("data"),
        ..Default::default()
    };
    drop(native_scan_service::open(config.clone(), false).unwrap());
    let (mut remote, _scan_recovery) = native_scan_service::open(config, true).unwrap();
    let request=HttpRequest { method:"POST".into(),path:"/mcp".into(),query:String::new(),headers:HashMap::new(),body:r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"diskgraph_scope","arguments":{"action":"list"}}}"#.into() };
    assert_eq!(
        http::handle_authenticated(&mut remote, &request, &HttpLimits::default(), None).status,
        401
    );
}
