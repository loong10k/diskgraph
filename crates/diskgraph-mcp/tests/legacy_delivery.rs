use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits, Security, ServerConfig};
use diskgraph_mcp::{McpConfig, McpService};
use serde_json::{Value, json};

struct Fixture {
    _directory: tempfile::TempDir,
    address: SocketAddr,
    token: String,
    service: McpService,
    principal: diskgraph_core::PrincipalId,
}

impl Fixture {
    fn new(max_response_bytes: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let service = McpService::open_remote(McpConfig {
            data_dir: directory.path().join("data"),
            ..McpConfig::default()
        })
        .unwrap();
        let key = b"isolated-legacy-delivery-test-key";
        let auth = Authenticator::new(AuthConfig::single("fixture", "diskgraph", key));
        let token = TokenMinter::new(key).mint(&TokenClaims {
            issuer: "fixture".into(),
            audience: "diskgraph".into(),
            subject: "reader".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        });
        // 隔离夹具由可信宿主显式授予权限；远程启动本身不会创建管理员。
        let principal = auth.authenticate(Some(&token)).unwrap().principal;
        service.engine().bootstrap_local_admin(&principal).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_service = service.clone();
        std::thread::spawn(move || {
            let _ = http::serve_config(
                server_service,
                listener,
                ServerConfig::modern(
                    HttpLimits {
                        max_response_bytes,
                        read_timeout: Duration::from_secs(3),
                        ..HttpLimits::default()
                    },
                    Security::local(Some(&auth)),
                )
                .with_legacy(),
                std::io::sink(),
            );
        });
        Self {
            _directory: directory,
            address,
            token,
            service,
            principal,
        }
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        stream
    }

    fn session(&self) -> (BufReader<TcpStream>, String) {
        let mut stream = self.connect();
        write!(
            stream,
            "GET /sse HTTP/1.1\r\nHost: fixture\r\nAuthorization: Bearer {}\r\n\r\n",
            self.token
        )
        .unwrap();
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        assert!(status.contains("200"), "{status}");
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if let Some(endpoint) = line.strip_prefix("data: /messages/") {
                return (reader, format!("/messages/{}", endpoint.trim()));
            }
        }
    }

    fn post(&self, endpoint: &str, body: &Value) -> (String, String) {
        let mut stream = self.connect();
        let body = body.to_string();
        write!(
            stream,
            "POST {endpoint} HTTP/1.1\r\nHost: fixture\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n{body}",
            self.token,
            body.len()
        )
        .unwrap();
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).unwrap();
        (status, String::from_utf8(bytes).unwrap())
    }
}

#[test]
fn grant_revocation_closes_the_legacy_stream_and_removes_its_delivery_session() {
    let fixture = Fixture::new(128);
    let (mut reader, endpoint) = fixture.session();
    fixture
        .service
        .engine()
        .control_store()
        .unwrap()
        .revoke_grant(
            &fixture.principal,
            &diskgraph_core::Permission::MetadataRead,
            &diskgraph_engine::admin_scope(),
        )
        .unwrap();
    let mut remaining = Vec::new();
    reader.read_to_end(&mut remaining).unwrap();
    let (status, body) = fixture.post(
        &endpoint,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    );
    assert!(status.contains("404"), "{status}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"],
        "unknown_session"
    );
}

#[test]
fn authenticated_legacy_responses_obey_the_configured_byte_limit() {
    let fixture = Fixture::new(128);
    let (mut reader, endpoint) = fixture.session();
    let (status, _) = fixture.post(
        &endpoint,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    );
    assert!(status.contains("202"), "{status}");
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if let Some(payload) = line.strip_prefix("data: ") {
            let payload = payload.trim_end();
            assert!(
                payload.len() <= 128,
                "legacy emitted {} bytes",
                payload.len()
            );
            let response: Value = serde_json::from_str(payload).unwrap();
            assert_eq!(response["id"], 1);
            assert_eq!(response["error"]["message"], "response_too_large");
            break;
        }
    }
}

#[test]
fn legacy_refuses_a_budget_that_cannot_fit_the_correlated_error_before_ack() {
    let fixture = Fixture::new(128);
    let (_reader, endpoint) = fixture.session();
    let (status, body) = fixture.post(
        &endpoint,
        &json!({"jsonrpc":"2.0","id":"x".repeat(256),"method":"tools/list"}),
    );
    assert!(status.contains("413"), "{status}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"],
        "response_budget_too_small"
    );
}
