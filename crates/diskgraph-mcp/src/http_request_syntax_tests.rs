//! 请求行与单值 MCP 头的真实 socket 负控；来源：RFC 9112 和 MCP 传输身份约束。
use crate::http::{HttpLimits, HttpRequest, read_request};
use std::io::{BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};

fn parse_wire(wire: &str) -> std::io::Result<Option<HttpRequest>> {
    parse_wire_bytes(wire.as_bytes())
}

fn parse_wire_bytes(wire: &[u8]) -> std::io::Result<Option<HttpRequest>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let mut client = TcpStream::connect(listener.local_addr()?)?;
    client.write_all(wire)?;
    let (server, _) = listener.accept()?;
    read_request(&mut BufReader::new(server), &HttpLimits::default())
}

#[test]
fn malformed_request_line_never_becomes_a_dispatchable_request() {
    for line in [
        "GET /mcp",
        "GET /mcp HTTP/9.9",
        "GET /mcp HTTP/1.1 extra",
        "GET /mcp\u{000b} HTTP/1.1",
        "GET /mcp\0 HTTP/1.1",
    ] {
        let wire = format!("{line}\r\nHost: localhost\r\n\r\n");
        let error = parse_wire(&wire).expect_err(line);
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{line:?}");
    }
}

#[test]
fn duplicate_mcp_session_and_protocol_headers_are_rejected_before_dispatch() {
    for name in ["Mcp-Session-Id", "MCP-Protocol-Version"] {
        for value in ["first", "different"] {
            let wire = format!(
                "GET /mcp HTTP/1.1\r\n{name}: first\r\n{}: {value}\r\n\r\n",
                name.to_ascii_lowercase()
            );
            let error = parse_wire(&wire).expect_err(&wire);
            assert_eq!(error.kind(), ErrorKind::InvalidData);
        }
    }
}

#[test]
fn supported_http_versions_preserve_request_and_single_value_headers() {
    for version in ["HTTP/1.0", "HTTP/1.1"] {
        let wire = format!(
            "GET /mcp?session=value {version}\r\nMcp-Session-Id: own-session\r\nMCP-Protocol-Version: 2025-03-26\r\n\r\n"
        );
        let request = parse_wire(&wire).unwrap().unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/mcp");
        assert_eq!(request.query, "session=value");
        assert_eq!(request.header("mcp-session-id"), Some("own-session"));
        assert_eq!(request.header("mcp-protocol-version"), Some("2025-03-26"));
    }
}

#[test]
fn invalid_utf8_body_is_rejected_before_dispatch_without_replacement() {
    let body = b"{\"method\":\"bad\xff\"}";
    let mut wire = format!(
        "POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    wire.extend_from_slice(body);
    let error = parse_wire_bytes(&wire)
        .expect_err("invalid body must not become a replacement-character request");
    assert_eq!(error.kind(), ErrorKind::InvalidData);
}

#[test]
fn valid_unicode_and_literal_replacement_character_remain_unchanged() {
    let body = "{\"method\":\"查询😀�\"}";
    let wire = format!(
        "POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(parse_wire(&wire).unwrap().unwrap().body, body);
}
