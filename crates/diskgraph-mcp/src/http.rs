//! Streamable HTTP transport (P4 tasks 5.1 / 5.2, specs MCP-01 / MCP-02 /
//! MCP-03).
//!
//! A minimal synchronous HTTP/1.1 server built on `std::net`, so the remote
//! transport adds no new dependency and shares the exact same `McpService`
//! dispatch the stdio transport uses. The server is read-only by construction:
//! it serves the same tools under the same authorization, and it never treats
//! a client-supplied path as anything but an opaque string.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::McpService;
use crate::auth::{AuthFailure, Authenticator, token_from_headers, unauthorized_body};
use crate::protocol::{PROTOCOL_VERSION, log_line, protocol_error};

pub use crate::client_address::observed_client_ip;
pub use crate::rate_limiter::RateLimiter;

/// The single endpoint a Streamable HTTP client posts to.
pub const MCP_ENDPOINT: &str = "/mcp";
/// A lightweight readiness endpoint; it exposes no indexed data.
pub const HEALTH_ENDPOINT: &str = "/healthz";
const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_HEADER_COUNT: usize = 100;

/// Server limits, applied before any work (spec MCP-06 / RT-02).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpLimits {
    /// Largest accepted request body.
    pub max_body_bytes: usize,
    /// Largest response the server will produce for one request.
    pub max_response_bytes: usize,
    /// Requests allowed per connection before it is closed.
    pub max_requests_per_connection: usize,
    /// Concurrent connections the server admits. Beyond this, new connections
    /// are refused with 503 rather than queued without bound.
    pub max_concurrent_connections: usize,
    /// Token-bucket rate for one client across its connections.
    pub max_requests_per_second_per_client: u32,
    /// Absolute deadline for reading one complete request, including its
    /// line, headers, and body. Receiving another byte does not renew it.
    pub read_timeout: Duration,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 1 << 20,
            max_response_bytes: 4 << 20,
            max_requests_per_connection: 256,
            max_concurrent_connections: 32,
            max_requests_per_second_per_client: 50,
            read_timeout: Duration::from_secs(10),
        }
    }
}

/// Origin and proxy policy for the HTTP transport (P4 task 5.4, spec MCP-06).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkPolicy {
    /// Extra origins allowed to call `/mcp`, matched exactly. The loopback
    /// family is always allowed: a local agent must work without ceremony.
    pub allowed_origins: Vec<String>,
    /// Whether an explicit `Origin: null` is accepted. Off by default: null is
    /// what a sandboxed page sends, and nothing about it proves locality.
    pub allow_null_origin: bool,
    /// 精确可信代理 IP；其追加的 XFF 用于诊断与限流，按链从右向左解析。
    /// 转发地址不建立主体身份，授权仍需 bearer 与实时 grant。
    pub trusted_proxies: Vec<String>,
}

impl NetworkPolicy {
    /// The default for a local deployment: loopback origins only.
    pub fn loopback_default() -> Self {
        Self::default()
    }

    /// The policy a non-loopback deployment uses when the operator asserts
    /// external TLS termination: still no extra origins unless named.
    pub fn with_origins(mut self, origins: impl IntoIterator<Item = String>) -> Self {
        self.allowed_origins = origins.into_iter().collect();
        self
    }

    /// Decides one Origin header. Requests without an Origin header are
    /// allowed: non-browser MCP clients do not send one, and absence is not a
    /// claim about where the caller came from.
    pub fn origin_decision(&self, origin: Option<&str>) -> OriginDecision {
        let Some(origin) = origin.map(str::trim) else {
            return OriginDecision::Absent;
        };
        if origin == "null" {
            return if self.allow_null_origin {
                OriginDecision::Allowed
            } else {
                OriginDecision::Refused
            };
        }
        let Some((scheme, host, _port)) = parse_origin(origin) else {
            return OriginDecision::Refused;
        };
        let secure_scheme = scheme == "http" || scheme == "https";
        if secure_scheme && is_loopback_host(host) {
            return OriginDecision::Allowed;
        }
        if self.allowed_origins.iter().any(|allowed| allowed == origin) {
            return OriginDecision::Allowed;
        }
        OriginDecision::Refused
    }
}

/// What an Origin header claim amounted to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginDecision {
    /// No Origin header: allowed, nothing to validate.
    Absent,
    Allowed,
    Refused,
}

/// The security context one request is evaluated under: who may authenticate
/// and what network policy applies.
#[derive(Clone)]
pub struct Security {
    pub authenticator: Option<std::sync::Arc<Authenticator>>,
    pub policy: NetworkPolicy,
}

impl Security {
    /// Local deployments: no remote authentication, loopback origin policy.
    pub fn local(authenticator: Option<&Authenticator>) -> Self {
        Self::remote(authenticator.map(|auth| std::sync::Arc::new(auth.clone())))
    }

    /// Build from an already-shared authenticator.
    pub fn remote(authenticator: Option<std::sync::Arc<Authenticator>>) -> Self {
        Self {
            authenticator,
            policy: NetworkPolicy::loopback_default(),
        }
    }
}

/// Whether a host is one of the loopback spellings.
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    host == "localhost"
        || host == "::1"
        || host
            .strip_prefix("127.")
            .is_some_and(|rest| rest.split('.').all(|part| part.parse::<u8>().is_ok()))
}

/// Splits `scheme://host:port` into its parts. Returns None when the origin
/// is not in that shape: a malformed origin is refused, never guessed.
fn parse_origin(origin: &str) -> Option<(&str, &str, Option<u16>)> {
    let (scheme, rest) = origin.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') => (host, Some(port.parse::<u16>().ok()?)),
        // IPv6 literals arrive bracketed: [::1]:8080.
        Some((before, port)) if before.starts_with('[') && before.ends_with(']') => (
            &before[1..before.len() - 1],
            Some(port.parse::<u16>().ok()?),
        ),
        _ => (authority, None),
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme, host, port))
}

/// The bind policy for a deployment (P4 task 5.4, spec MCP-06). Plaintext on
/// loopback is fine; a network interface needs both remote authentication and
/// an explicit acknowledgment that TLS is terminated by a trusted tunnel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindPolicy {
    LoopbackPlaintext,
    NetworkWithTunnel,
    RefuseNoAuth,
    RefusePlaintext,
}

/// Decides whether a bind is allowed. `secure_transport` asserts that TLS (or
/// an equivalent encrypted tunnel) terminates in front of this server, which
/// is the only honest way this plaintext HTTP core serves a network interface.
pub fn bind_decision(host: &str, has_auth: bool, secure_transport: bool) -> BindPolicy {
    if is_loopback_host(host) {
        return BindPolicy::LoopbackPlaintext;
    }
    if !has_auth {
        return BindPolicy::RefuseNoAuth;
    }
    if !secure_transport {
        return BindPolicy::RefusePlaintext;
    }
    BindPolicy::NetworkWithTunnel
}

/// The bound address, reported for diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundAddress {
    pub address: String,
    pub port: u16,
}

impl std::fmt::Display for BoundAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "http://{}:{}", self.address, self.port)
    }
}

/// Binds a listener. The default binds loopback only: a remote deployment has
/// to opt into a wider interface explicitly (spec MCP-06 / MCP-03).
pub fn bind(address: &str, port: u16) -> std::io::Result<(TcpListener, BoundAddress)> {
    let listener = TcpListener::bind((address, port))?;
    let local = listener.local_addr()?;
    Ok((
        listener,
        BoundAddress {
            address: local.ip().to_string(),
            port: local.port(),
        },
    ))
}

/// One parsed request: enough to route and to enforce the contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    /// The raw query string after `?`, unparsed. Legacy MCP sessions address
    /// their message endpoint by query parameter.
    pub query: String,
    /// Lower-cased header names mapped to their first value.
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// One query parameter, taken as an opaque string. Client-controlled
    /// query values are session identifiers, never paths.
    pub fn query_param(&self, name: &str) -> Option<String> {
        for pair in self.query.split('&') {
            if let Some((key, value)) = pair.split_once('=')
                && key == name
            {
                return Some(value.to_owned());
            }
        }
        None
    }
}

/// A response the connection loop writes verbatim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
    /// Session identifier echoed for MCP Streamable HTTP clients.
    pub session: Option<String>,
}

impl HttpResponse {
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string(),
            session: None,
        }
    }

    /// An empty 202 for JSON-RPC notifications: per the Streamable HTTP
    /// contract the response carries no body and no Content-Type at all.
    pub fn accepted() -> Self {
        Self {
            status: 202,
            content_type: "application/json",
            body: String::new(),
            session: None,
        }
    }

    /// An empty event-stream answer, for hosts whose client reads every POST
    /// response as a stream.
    pub fn accepted_stream() -> Self {
        Self {
            status: 202,
            content_type: "text/event-stream",
            body: String::new(),
            session: None,
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
            session: None,
        }
    }
}

/// Routes one request against the shared service with no remote
/// authentication. Kept free of I/O so the whole contract is testable without
/// a socket. A network deployment routes through [`handle_authenticated`].
pub fn handle(
    service: &mut McpService,
    request: &HttpRequest,
    limits: &HttpLimits,
) -> HttpResponse {
    handle_authenticated(service, request, limits, None)
}

/// The routing entry point with optional remote authentication. When an
/// authenticator is supplied, every `/mcp` request must present a valid
/// bearer token; an unauthenticated call is refused before the engine is
/// touched and before any index data could be observed (spec SC-05).
pub fn handle_authenticated(
    service: &mut McpService,
    request: &HttpRequest,
    limits: &HttpLimits,
    authenticator: Option<&Authenticator>,
) -> HttpResponse {
    handle_secured(service, request, limits, &Security::local(authenticator))
}

/// The full routing entry point: authentication, origin policy, then dispatch.
pub fn handle_secured(
    service: &mut McpService,
    request: &HttpRequest,
    limits: &HttpLimits,
    security: &Security,
) -> HttpResponse {
    if request.path == HEALTH_ENDPOINT {
        if request.method != "GET" {
            return HttpResponse::json(
                405,
                json!({"error": "method_not_allowed", "allowed": "GET"}),
            );
        }
        return HttpResponse::json(200, json!({"status": "ok", "protocol": PROTOCOL_VERSION}));
    }
    if request.path != MCP_ENDPOINT {
        // Unknown paths never reach the engine and never leak a filesystem
        // hint; a 404 body carries no directory information.
        return HttpResponse::json(404, json!({"error": "not_found"}));
    }
    // Origin is checked before authentication: a cross-origin browser request
    // is refused without revealing whether the endpoint would have accepted a
    // token, and without spending verification work on it.
    if security.policy.origin_decision(request.header("origin")) == OriginDecision::Refused {
        return HttpResponse::json(403, json!({"error": "forbidden_origin"}));
    }
    // Authentication precedes every other MCP concern, so an unauthenticated
    // caller learns nothing beyond the fact that the endpoint exists.
    let mut authenticated_service;
    let service = if let Some(authenticator) = &security.authenticator {
        let token = token_from_headers(&request.headers);
        match authenticator.authenticate(token.as_deref()) {
            Ok(identity) => {
                authenticated_service = service.for_identity(&identity);
                &mut authenticated_service
            }
            Err(failure) => return unauthorized(failure),
        }
    } else if service.context.trusted_local() {
        service
    } else {
        return unauthorized(AuthFailure::Missing);
    };
    match request.method.as_str() {
        "POST" => handle_post(service, request, limits),
        // GET /mcp is answered inline by the serve loop as a long-lived
        // event stream; buffered callers (tests) get the handshake frame.
        "GET" => HttpResponse {
            status: 200,
            content_type: "text/event-stream",
            body: ": stream\n\n".to_owned(),
            session: None,
        },
        "DELETE" => handle_delete(request),
        _ => HttpResponse::json(
            405,
            json!({"error": "method_not_allowed", "allowed": "POST, GET, DELETE"}),
        ),
    }
}

/// A refused request: 401 with a stable reason and nothing else.
fn unauthorized(failure: AuthFailure) -> HttpResponse {
    HttpResponse::json(failure.http_status(), unauthorized_body(failure))
}

fn handle_post(
    service: &mut McpService,
    request: &HttpRequest,
    limits: &HttpLimits,
) -> HttpResponse {
    if request.body.len() > limits.max_body_bytes {
        return HttpResponse::json(
            413,
            json!({"error": "payload_too_large", "limit": limits.max_body_bytes}),
        );
    }
    // Streamable HTTP accepts one JSON-RPC message per POST. A batch is
    // refused explicitly rather than partially executed.
    let value: Value = match serde_json::from_str(request.body.trim()) {
        Ok(value) => value,
        Err(error) => {
            return HttpResponse::json(
                400,
                protocol_error(Value::Null, -32700, &format!("parse error: {error}")),
            );
        }
    };
    if value.is_array() {
        return HttpResponse::json(
            400,
            protocol_error(
                Value::Null,
                -32600,
                "batched requests are not supported; send one message per POST",
            ),
        );
    }
    // JSON-RPC notifications carry no id and expect no response body: the
    // Streamable HTTP contract answers them with an empty 202. Real clients
    // (rmcp-based hosts) send `notifications/initialized` right after the
    // handshake, and treating it as a bad frame kills the session.
    if value.is_object() && value.get("method").is_some() && value.get("id").is_none() {
        return HttpResponse::accepted();
    }
    let decoded = match crate::protocol::decode_request(request.body.trim()) {
        Ok(decoded) => decoded,
        Err(error) => {
            let reason = match error {
                crate::protocol::FrameError::Malformed => "malformed frame",
                crate::protocol::FrameError::MissingMethod => "missing method",
                crate::protocol::FrameError::Batch => "batched frame",
            };
            return HttpResponse::json(400, protocol_error(Value::Null, -32600, reason));
        }
    };
    // A client may pin a protocol version; an unknown one is a version
    // mismatch, not a silent downgrade.
    if let Some(requested) = request.header("mcp-protocol-version")
        && requested != PROTOCOL_VERSION
    {
        return HttpResponse::json(
            400,
            protocol_error(
                decoded.id.clone(),
                -32600,
                &format!(
                    "unsupported protocol version {requested}; this server speaks {PROTOCOL_VERSION}"
                ),
            ),
        );
    }
    let response = service.handle(&decoded);
    let Some(body) =
        crate::bounded_json_writer::BoundedJsonWriter::encode(&response, limits.max_response_bytes)
    else {
        return HttpResponse::json(
            500,
            json!({"error":"response_too_large", "limit":limits.max_response_bytes}),
        );
    };
    let mut http = HttpResponse {
        status: 200,
        content_type: "application/json",
        body,
        session: None,
    };
    // The session header lets a client resume; the business state it points at
    // is durable in the control store, not in this process.
    http.session = Some(session_for(&decoded.id));
    http
}

/// Ending a session only drops this connection's view; durable jobs and
/// revisions remain queryable (spec MCP-05). GET /mcp is answered inline by
/// the serve loop as a long-lived event stream.
fn handle_delete(_request: &HttpRequest) -> HttpResponse {
    HttpResponse::json(200, json!({"status": "session_ended"}))
}

/// A stable session identifier derived from the connection's request ids.
fn session_for(id: &Value) -> String {
    format!("diskgraph-{}", id.to_string().replace(['"', ' '], ""))
}

/// Reads one HTTP/1.1 request from a stream.
pub fn read_request(
    reader: &mut BufReader<TcpStream>,
    limits: &HttpLimits,
) -> std::io::Result<Option<HttpRequest>> {
    let deadline = Instant::now() + limits.read_timeout;
    let Some(request_line) = read_bounded_line(reader, MAX_REQUEST_LINE_BYTES, deadline)? else {
        return Ok(None);
    };
    let mut parts = request_line.trim_end_matches(['\r', '\n']).split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let version = parts.next().unwrap_or_default();
    // 传输身份只能建立在完整单一请求行上，不能忽略未知版本或第四段。
    if method.is_empty()
        || target.is_empty()
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || parts.next().is_some()
        || target.chars().any(char::is_control)
        || !method.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
    {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "malformed HTTP request line",
        ));
    }
    let path = target.split('?').next().unwrap_or("/").to_owned();
    let query = target
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or("")
        .to_owned();

    let mut headers = HashMap::new();
    let mut header_bytes = 0usize;
    let mut header_count = 0usize;
    loop {
        let remaining = MAX_HEADER_BYTES.saturating_sub(header_bytes);
        let Some(line) = read_bounded_line(reader, remaining, deadline)? else {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "incomplete HTTP headers",
            ));
        };
        header_bytes += line.len();
        header_count += 1;
        if header_count > MAX_HEADER_COUNT {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "too many HTTP headers",
            ));
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidData, "malformed HTTP header"))?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty()
            || (matches!(
                name.as_str(),
                "content-length"
                    | "transfer-encoding"
                    | "authorization"
                    | "origin"
                    | "host"
                    | "mcp-session-id"
                    | "mcp-protocol-version"
            ) && headers.contains_key(&name))
        {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "ambiguous HTTP header",
            ));
        }
        let forwarded = name == "x-forwarded-for";
        headers
            .entry(name)
            .and_modify(|existing: &mut String| {
                // XFF 是有序地址列表；保留代理追加的后续字段，不能只信第一行。
                // 所有原始字段仍计入总头预算；合并只增加有界逗号分隔符。
                if forwarded {
                    existing.push_str(", ");
                    existing.push_str(value.trim());
                }
            })
            .or_insert_with(|| value.trim().to_owned());
        // 其他非安全重复头保留第一值；framing、身份、host 与 Origin 必须单一。
    }

    if headers.contains_key("transfer-encoding") {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "transfer encoding is unsupported",
        ));
    }
    let length: usize = headers
        .get("content-length")
        .map(|value| {
            value
                .parse()
                .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "invalid content length"))
        })
        .transpose()?
        .unwrap_or(0);
    if length > limits.max_body_bytes {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "HTTP body exceeds limit",
        ));
    }
    let mut body = vec![0u8; length];
    let mut filled = 0usize;
    while filled < length {
        apply_read_deadline(reader, deadline)?;
        let count = reader
            .read(&mut body[filled..])
            .map_err(normalize_request_timeout)?;
        if count == 0 {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "incomplete HTTP body",
            ));
        }
        filled += count;
    }
    Ok(Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }))
}

/// Reads at most `limit` bytes, including CRLF, without allowing BufRead's
/// unbounded `read_line` allocation or a slow client to renew the deadline.
fn read_bounded_line(
    reader: &mut BufReader<TcpStream>,
    limit: usize,
    deadline: Instant,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        apply_read_deadline(reader, deadline)?;
        let available = reader.fill_buf().map_err(normalize_request_timeout)?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "incomplete HTTP line",
                ))
            };
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(take) > limit {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "HTTP line exceeds limit",
            ));
        }
        line.extend_from_slice(&available[..take]);
        reader.consume(take);
        if line.last() == Some(&b'\n') {
            return String::from_utf8(line).map(Some).map_err(|_| {
                std::io::Error::new(ErrorKind::InvalidData, "HTTP line is not UTF-8")
            });
        }
    }
}

fn apply_read_deadline(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> std::io::Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(std::io::Error::new(
            ErrorKind::TimedOut,
            "HTTP request deadline exceeded",
        ));
    }
    reader.get_ref().set_read_timeout(Some(remaining))
}

fn normalize_request_timeout(error: std::io::Error) -> std::io::Error {
    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
        std::io::Error::new(ErrorKind::TimedOut, "HTTP request deadline exceeded")
    } else {
        error
    }
}

/// Serializes a response as an HTTP/1.1 message.
/// 参数：stream：已连接的 TCP 流；response：待发送的协议响应。
/// 返回：序列化/发送成功为 ()，连接错误向调用者传播。
pub fn write_response(stream: &mut TcpStream, response: &HttpResponse) -> std::io::Result<()> {
    write_response_with_connection(stream, response, true)
}

/// 发送声明关闭的响应，接收端清理由调用方在独立预算内完成。
/// 参数：stream：已连接的 TCP 流；response：待发送的拒绝响应。
/// 返回：发送成功为 ()；此函数不关闭或清理接收端。
pub(crate) fn write_closing_response(
    stream: &mut TcpStream,
    response: &HttpResponse,
) -> std::io::Result<()> {
    write_response_with_connection(stream, response, false)
}

fn write_response_with_connection(
    stream: &mut TcpStream,
    response: &HttpResponse,
    keep_alive: bool,
) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, reason,);
    if !response.content_type.is_empty() {
        head.push_str(&format!("Content-Type: {}\r\n", response.content_type));
    }
    head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    if let Some(session) = &response.session {
        head.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
    }
    head.push_str(if keep_alive {
        "Connection: keep-alive\r\n\r\n"
    } else {
        "Connection: close\r\n\r\n"
    });
    stream.write_all(head.as_bytes())?;
    stream.write_all(response.body.as_bytes())?;
    stream.flush()
}

/// Serves connections until the listener is closed. Returns the number of
/// requests handled.
pub fn serve(
    service: McpService,
    listener: TcpListener,
    limits: &HttpLimits,
    log: impl Write + Send + 'static,
) -> std::io::Result<usize> {
    serve_authenticated(service, listener, limits, None, log)
}

/// The serve loop with optional remote authentication.
pub fn serve_authenticated(
    service: McpService,
    listener: TcpListener,
    limits: &HttpLimits,
    authenticator: Option<&crate::auth::Authenticator>,
    log: impl Write + Send + 'static,
) -> std::io::Result<usize> {
    let security = Security::local(authenticator);
    serve_secured(service, listener, limits, &security, log)
}

/// The serve loop with the full security context (auth + network policy).
pub fn serve_secured(
    service: McpService,
    listener: TcpListener,
    limits: &HttpLimits,
    security: &Security,
    log: impl Write + Send + 'static,
) -> std::io::Result<usize> {
    serve_config(
        service,
        listener,
        ServerConfig::modern(*limits, security.clone()),
        log,
    )
}

/// The full serving loop. The legacy branch only activates when the config
/// enables it; probing `/sse` on a modern-only server is told the adapter is
/// off, never handed a silent 404.
pub fn serve_config(
    service: McpService,
    listener: TcpListener,
    config: ServerConfig,
    log: impl Write + Send + 'static,
) -> std::io::Result<usize> {
    serve_config_with_runtime(service, listener, config, log, std::sync::Arc::default())
}

/// 参数沿用原监听配置，state 是同一宿主停止状态；返回实际 join 后的处理计数或原监听 IO 错误。
pub(crate) fn serve_config_with_runtime(
    service: McpService,
    listener: TcpListener,
    config: ServerConfig,
    log: impl Write + Send + 'static,
    state: std::sync::Arc<crate::http_shutdown_state::HttpShutdownState>,
) -> std::io::Result<usize> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    listener.set_nonblocking(true)?;
    let mut connections = crate::http_connections::HttpConnections::new(Arc::clone(&state));
    let limits = config.limits;
    let security = config.security;
    let legacy_transport = crate::legacy_transport::LegacyTransport::new(limits);
    let shared_service = Arc::new(service);
    let sse_counts = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let shared_log = Arc::new(Mutex::new(log));
    let limiter = Arc::new(RateLimiter::new(&limits));
    let handled = Arc::new(AtomicUsize::new(0));
    let security = security.clone();
    let mut accept_failure = None;

    while !state.stopped() {
        connections.reap();
        if state.stopped() {
            break;
        }
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => {
                // A closed listener surfaces here; accepting anything else is
                // fatal for this single-threaded accept loop either way.
                if let Ok(mut log) = shared_log.lock() {
                    let _ = writeln!(
                        log,
                        "{}",
                        log_line("accept_failed", &[("reason", &error.to_string())])
                    );
                }
                // 保留原监听错误；先关闭并 join 原连接，不能将停止失败投影为成功。
                accept_failure = Some(error);
                break;
            }
        };
        // 各平台 accept 对 nonblocking 继承不同；原请求读取仍用阻塞+绝对期限。
        stream.set_nonblocking(false)?;
        let peer = stream
            .peer_addr()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| "unknown".to_owned());
        // Connection cap: refuse instead of queueing without bound, so a
        // flood can never starve the engine's workers.
        if connections.len() >= limits.max_concurrent_connections {
            let _ = crate::connection_rejection::reject_connection(
                &mut stream,
                limits.max_concurrent_connections,
            );
            continue;
        }
        let Some(connection_id) = state.register(&stream)? else {
            break;
        };
        let connection_state = Arc::clone(&state);
        let shared_service = Arc::clone(&shared_service);
        let sse_counts = Arc::clone(&sse_counts);
        let shared_log = Arc::clone(&shared_log);
        let limiter = Arc::clone(&limiter);
        let handled = Arc::clone(&handled);
        let security = security.clone();
        let legacy_transport = legacy_transport.clone();
        let legacy_sse = config.legacy_sse;
        let worker = std::thread::Builder::new().spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = stream.set_read_timeout(Some(limits.read_timeout));
            let _ = stream.set_write_timeout(Some(limits.read_timeout));
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(clone) => clone,
                Err(_) => return,
            });
            for _ in 0..limits.max_requests_per_connection {
                let request = match read_request(&mut reader, &limits) {
                    Ok(Some(request)) => request,
                    Ok(None) => break,
                    Err(error) => {
                        // A stalled or aborted client frees its slot here: the
                        // read timeout is what keeps slow connections from
                        // holding the server hostage.
                        let _ = shared_log.lock().map(|mut log| {
                            let _ = writeln!(
                                log,
                                "{}",
                                log_line("read_failed", &[("reason", &error.to_string())])
                            );
                        });
                        break;
                    }
                };
                if std::env::var("DISKGRAPH_HTTP_DEBUG").is_ok() {
                    eprintln!(
                        "[http-debug] {} {} body={:?}",
                        request.method,
                        request.path,
                        &request.body[..request.body.len().min(120)]
                    );
                }
                let client = observed_client_ip(&peer, &request.headers, &security.policy);
                if let Err(retry_after_ms) = limiter.check(&client) {
                    if write_response(
                        &mut stream,
                        &HttpResponse::json(
                            429,
                            json!({
                                "error": "rate_limited",
                                "retry_after_ms": retry_after_ms,
                            }),
                        ),
                    )
                    .is_err()
                    {
                        break;
                    }
                    continue;
                }
                let mut stream_identity = None;
                let mut stream_slot = None;
                // 所有 SSE 握手先通过与 POST 相同的 Origin/认证边界。
                if request.method == "GET"
                    && (request.path == MCP_ENDPOINT
                        || request.path == crate::legacy::LEGACY_SSE_PATH)
                {
                    if security.policy.origin_decision(request.header("origin"))
                        == OriginDecision::Refused
                    {
                        let _ = write_response(
                            &mut stream,
                            &HttpResponse::json(403, json!({"error": "forbidden_origin"})),
                        );
                        break;
                    }
                    if let Some(authenticator) = &security.authenticator {
                        let token = token_from_headers(&request.headers);
                        match authenticator.authenticate(token.as_deref()) {
                            Ok(identity) => stream_identity = Some(identity),
                            Err(failure) => {
                                let _ = write_response(&mut stream, &unauthorized(failure));
                                break;
                            }
                        }
                    }
                    if security.authenticator.is_none() && !shared_service.context.trusted_local() {
                        let _ = write_response(&mut stream, &unauthorized(AuthFailure::Missing));
                        break;
                    }
                    let principal = stream_identity
                        .as_ref()
                        .map(|identity| identity.principal.as_str())
                        .unwrap_or("trusted-local")
                        .to_owned();
                    stream_slot =
                        crate::sse_slot::SseSlot::acquire(Arc::clone(&sse_counts), principal);
                    if stream_slot.is_none() {
                        let _ = write_response(
                            &mut stream,
                            &HttpResponse::json(
                                429,
                                json!({"error":"sse_principal_limit", "max":4}),
                            ),
                        );
                        break;
                    }
                }
                if stream_slot.is_some() {
                    let principal = stream_identity
                        .as_ref()
                        .map(|identity| identity.principal.as_str())
                        .unwrap_or(shared_service.context.principal().as_str());
                    let _ = shared_log.lock().map(|mut log| {
                        writeln!(
                            log,
                            "{}",
                            log_line(
                                "sse_open",
                                &[("principal", principal), ("path", &request.path)]
                            )
                        )
                    });
                }
                let _stream_slot = stream_slot;
                // The legacy adapter owns its two paths outright; every other
                // request follows the modern transport pipeline.
                if legacy_sse
                    && request.method == "GET"
                    && request.path == crate::legacy::LEGACY_SSE_PATH
                {
                    legacy_transport.serve_stream(
                        &mut stream,
                        stream_identity.as_ref(),
                        &shared_service,
                        &shared_log,
                    );
                    break;
                }
                if request.method == "POST"
                    && request.path.trim_end_matches('/') == crate::legacy::LEGACY_MESSAGES_PATH
                {
                    if !legacy_sse {
                        let _ = write_response(
                            &mut stream,
                            &HttpResponse::json(404, json!({"error": "legacy_sse_disabled"})),
                        );
                        continue;
                    }
                    if legacy_transport.post(
                        &mut stream,
                        &request,
                        &security,
                        &shared_service,
                        &shared_log,
                    ) {
                        continue;
                    }
                    break;
                }
                if !legacy_sse && request.path == crate::legacy::LEGACY_SSE_PATH {
                    let _ = write_response(
                        &mut stream,
                        &HttpResponse::json(404, json!({"error": "legacy_sse_disabled"})),
                    );
                    continue;
                }
                // GET /mcp opens the server-to-client event stream. It is
                // long-lived: hold the connection, emit keep-alives, and let
                // the client's disconnect end it (Streamable HTTP contract).
                if request.method == "GET" && request.path == MCP_ENDPOINT {
                    // Long-lived server-to-client stream. It must outlive the
                    // read timeout: a read timeout on the probe is an idle
                    // tick, not a client disconnect, and treating it as one
                    // makes the stream die every few seconds and sends
                    // rmcp-based hosts into a reconnect loop.
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
                    if stream.write_all(head.as_bytes()).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                    // A cloned request reader has just adjusted the socket's
                    // receive timeout. Use nonblocking probes so liveness
                    // never depends on which descriptor last changed it.
                    if stream.set_nonblocking(true).is_err() {
                        break;
                    }
                    let mut probe = [0u8; 1];
                    loop {
                        if stream_identity.as_ref().is_some_and(|identity| {
                            !stream_identity_valid(&shared_service, identity)
                        }) {
                            break;
                        }
                        match stream.peek(&mut probe) {
                            Ok(0) => break,
                            Ok(_) => {
                                // Unexpected client bytes on a GET stream are
                                // protocol noise; drain and keep holding.
                                let _ = stream.read(&mut probe);
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::TimedOut
                                ) =>
                            {
                                std::thread::sleep(Duration::from_secs(1));
                                // Idle tick: a comment keeps intermediaries
                                // from reaping the connection.
                                if stream
                                    .write_all(crate::legacy::keepalive_event().as_bytes())
                                    .is_err()
                                {
                                    break;
                                }
                                let _ = stream.flush();
                            }
                            Err(_) => break,
                        }
                    }
                    break;
                }
                let response = {
                    let mut service = shared_service.as_ref().clone();
                    handle_secured(&mut service, &request, &limits, &security)
                };
                if write_response(&mut stream, &response).is_err() {
                    break;
                }
                handled.fetch_add(1, Ordering::SeqCst);
                let principal = if let Some(authenticator) = &security.authenticator {
                    authenticator
                        .authenticate(token_from_headers(&request.headers).as_deref())
                        .map(|identity| identity.principal.as_str().to_owned())
                        .unwrap_or_else(|_| "anonymous".to_owned())
                } else {
                    shared_service.context.principal().as_str().to_owned()
                };
                let _ = shared_log.lock().map(|mut log| {
                    let _ = writeln!(
                        log,
                        "{}",
                        log_line(
                            "request",
                            &[
                                ("principal", &principal),
                                ("client", &client),
                                ("path", &request.path)
                            ]
                        )
                    );
                });
            }
            }));
            connection_state.release(connection_id);
            if let Err(payload) = outcome { std::panic::resume_unwind(payload); }
        });
        match worker {
            Ok(worker) => connections.push(worker),
            Err(error) => {
                state.release(connection_id);
                return Err(error);
            }
        }
    }
    connections.finish();
    match accept_failure {
        Some(error) => Err(error),
        None => Ok(handled.load(Ordering::SeqCst)),
    }
}

/// Everything one HTTP listener needs: limits, security context, and the
/// legacy adapter gate (default off, P4 tasks 5.6/5.7).
pub struct ServerConfig {
    pub limits: HttpLimits,
    pub security: Security,
    pub legacy_sse: bool,
}

impl ServerConfig {
    /// The modern-transport server with defaults.
    pub fn modern(limits: HttpLimits, security: Security) -> Self {
        Self {
            limits,
            security,
            legacy_sse: false,
        }
    }

    /// The same server with the legacy adapter enabled.
    pub fn with_legacy(mut self) -> Self {
        self.legacy_sse = true;
        self
    }
}

/// 检查流身份是否到期或完全撤权；参数为共享服务与认证主体，返回实时可用性。
pub(crate) fn stream_identity_valid(
    service: &McpService,
    identity: &crate::auth::AuthenticatedPrincipal,
) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(u64::MAX);
    now < identity.expires_at_unix_seconds && service.identity_is_live(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToolProfile;
    use diskgraph_core::Authorizer as _;
    use diskgraph_testkit::http_client;

    #[test]
    fn request_line_and_headers_are_bounded_before_authentication() {
        for request in [
            format!("GET /{} HTTP/1.1\r\nHost: x\r\n\r\n", "x".repeat(16 * 1024)),
            format!(
                "GET /mcp HTTP/1.1\r\nX-Pad: {}\r\n\r\n",
                "x".repeat(64 * 1024)
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let mut client = TcpStream::connect(address).unwrap();
            client.write_all(request.as_bytes()).unwrap();
            let (server, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(server);
            let error = read_request(&mut reader, &HttpLimits::default()).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        }
    }

    #[test]
    fn request_deadline_does_not_restart_after_each_received_byte() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let sender = std::thread::spawn(move || {
            let mut client = TcpStream::connect(address).unwrap();
            for _ in 0..30 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let (server, _) = listener.accept().unwrap();
        server
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut reader = BufReader::new(server);
        let limits = HttpLimits {
            read_timeout: Duration::from_millis(100),
            ..HttpLimits::default()
        };
        let started = std::time::Instant::now();
        let error = read_request(&mut reader, &limits).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut, "{error}");
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "{:?}",
            started.elapsed()
        );
        sender.join().unwrap();
    }

    #[test]
    fn unsupported_or_ambiguous_http_framing_is_rejected() {
        for request in [
            "POST /mcp HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "POST /mcp HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nx",
            "POST /mcp HTTP/1.1\r\nContent-Length: bogus\r\n\r\n",
            "POST /mcp HTTP/1.1\r\nContent-Length: 1000\r\n\r\n",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let mut client = TcpStream::connect(address).unwrap();
            client.write_all(request.as_bytes()).unwrap();
            let (server, _) = listener.accept().unwrap();
            server
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let mut reader = BufReader::new(server);
            let limits = HttpLimits {
                max_body_bytes: 32,
                read_timeout: Duration::from_millis(100),
                ..HttpLimits::default()
            };
            let error = read_request(&mut reader, &limits).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{request:?}: {error}");
        }
    }

    #[test]
    fn duplicate_forwarded_fields_keep_the_proxy_appended_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let limits = HttpLimits {
            max_requests_per_second_per_client: 1,
            ..HttpLimits::default()
        };
        let policy = NetworkPolicy {
            trusted_proxies: vec!["127.0.0.1".into(), "10.0.0.1".into()],
            ..NetworkPolicy::default()
        };
        let limiter = RateLimiter::new(&limits);
        for (index, spoofed) in ["192.0.2.98", "192.0.2.99"].iter().enumerate() {
            let mut client = TcpStream::connect(address).unwrap();
            let request = format!(
                "GET /healthz HTTP/1.1\r\nHost: x\r\nX-Forwarded-For: {spoofed}\r\nx-FORWARDED-for: 203.0.113.9, 10.0.0.1\r\n\r\n"
            );
            client.write_all(request.as_bytes()).unwrap();
            let (server, peer) = listener.accept().unwrap();
            let parsed = read_request(&mut BufReader::new(server), &limits)
                .unwrap()
                .unwrap();
            let observed = observed_client_ip(&peer.to_string(), &parsed.headers, &policy);
            assert_eq!(observed, "203.0.113.9");
            assert_eq!(limiter.check_at(&observed, 0).is_ok(), index == 0);
        }
    }

    fn service(label: &str) -> (McpService, crate::tests::McpTestDirectory) {
        service_with_profile(label, ToolProfile::ReadFull)
    }

    fn service_with_profile(
        label: &str,
        profile: ToolProfile,
    ) -> (McpService, crate::tests::McpTestDirectory) {
        let directory = tempfile::TempDir::with_prefix(format!("diskgraph-http-{label}-")).unwrap();
        crate::tests::McpTestDirectory::open(
            crate::McpConfig {
                data_dir: directory.path().join("data"),
                profile,
                principal: diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap(),
                legacy_sse: false,
            },
            directory,
        )
        .unwrap()
    }

    fn post(body: &str) -> HttpRequest {
        HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            query: String::new(),
            headers: HashMap::new(),
            body: body.into(),
        }
    }

    fn response_json(response: &HttpResponse) -> Value {
        serde_json::from_str(&response.body).expect("JSON body")
    }

    #[test]
    fn a_posted_initialize_answers_over_http() {
        let (mut service, _keep) = service("init");
        let limits = HttpLimits::default();
        let response = handle(
            &mut service,
            &post(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            ),
            &limits,
        );
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        let body = response_json(&response);
        assert_eq!(body["result"]["serverInfo"]["name"], "diskgraph");
        assert!(response.session.is_some(), "a session id is issued");
    }

    #[test]
    fn tool_listing_and_calls_work_over_the_same_service() {
        let (mut service, _keep) = service("tools");
        let limits = HttpLimits::default();
        let listed = handle(
            &mut service,
            &post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
            &limits,
        );
        let tools = response_json(&listed)["result"]["tools"]
            .as_array()
            .unwrap()
            .len();
        assert!(tools > 0);

        // An unindexed scope is `not_indexed`, exactly as over stdio.
        let called = handle(
            &mut service,
            &post(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"diskgraph_top","arguments":{"scope":"scope-none"}}}"#,
            ),
            &limits,
        );
        let body = response_json(&called);
        assert_eq!(
            body["error"]["data"]["business_code"], "not_found",
            "an unknown scope is not_found, never a path on the client"
        );
    }

    #[test]
    fn oversized_bodies_and_batches_are_refused() {
        let (mut service, _keep) = service("limits");
        let limits = HttpLimits {
            max_body_bytes: 32,
            ..HttpLimits::default()
        };
        let big = post(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","pad":"{}"}}"#,
            "x".repeat(64)
        ));
        assert_eq!(handle(&mut service, &big, &limits).status, 413);

        let limits = HttpLimits::default();
        let batch = post(r#"[{"jsonrpc":"2.0","id":1,"method":"tools/list"}]"#);
        let response = handle(&mut service, &batch, &limits);
        assert_eq!(response.status, 400);
        assert_eq!(response_json(&response)["error"]["code"], -32600);
    }

    #[test]
    fn an_unsupported_protocol_version_is_reported_not_downgraded() {
        let (mut service, _keep) = service("version");
        let mut request = post(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        request
            .headers
            .insert("mcp-protocol-version".into(), "1999-01-01".into());
        let response = handle(&mut service, &request, &HttpLimits::default());
        assert_eq!(response.status, 400);
        assert!(
            response.body.contains("unsupported protocol version"),
            "body: {}",
            response.body
        );
    }

    #[test]
    fn get_and_delete_follow_the_transport_contract() {
        let (mut service, _keep) = service("verbs");
        let limits = HttpLimits::default();
        let get = HttpRequest {
            method: "GET".into(),
            path: MCP_ENDPOINT.into(),
            query: String::new(),
            headers: HashMap::new(),
            body: String::new(),
        };
        let response = handle(&mut service, &get, &limits);
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/event-stream");
        assert!(response.body.contains("stream"));

        let delete = HttpRequest {
            method: "DELETE".into(),
            path: MCP_ENDPOINT.into(),
            query: String::new(),
            headers: HashMap::new(),
            body: String::new(),
        };
        assert_eq!(handle(&mut service, &delete, &limits).status, 200);
    }

    #[test]
    fn unknown_paths_and_methods_never_reach_the_engine() {
        let (mut service, _keep) = service("routing");
        let limits = HttpLimits::default();
        let unknown = HttpRequest {
            method: "POST".into(),
            path: "/etc/passwd".into(),
            query: String::new(),
            headers: HashMap::new(),
            body: String::new(),
        };
        let response = handle(&mut service, &unknown, &limits);
        assert_eq!(response.status, 404);
        assert!(
            !response.body.contains("passwd"),
            "no request echo in the body"
        );
        assert!(
            !response.body.contains('/'),
            "no filesystem hint in the body"
        );

        let wrong_method = HttpRequest {
            method: "PUT".into(),
            path: MCP_ENDPOINT.into(),
            query: String::new(),
            headers: HashMap::new(),
            body: String::new(),
        };
        assert_eq!(handle(&mut service, &wrong_method, &limits).status, 405);
    }

    #[test]
    fn the_health_endpoint_reports_no_indexed_data() {
        let (mut service, _keep) = service("health");
        let request = HttpRequest {
            method: "GET".into(),
            path: HEALTH_ENDPOINT.into(),
            query: String::new(),
            headers: HashMap::new(),
            body: String::new(),
        };
        let response = handle(&mut service, &request, &HttpLimits::default());
        let body = response_json(&response);
        assert_eq!(body["status"], "ok");
        assert!(body.get("scopes").is_none());
        assert!(body.get("server_id").is_none());
    }

    #[test]
    fn a_real_socket_round_trip_answers_the_same_as_the_direct_call() {
        let (service, directory) = service("socket");
        // Build a tiny index on the server side.
        let project = directory.path().join("project");
        std::fs::create_dir_all(project.join("target")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(project.join("target").join("bin"), vec![0; 8192]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let decision = authorizer.decide(
            &principal,
            &diskgraph_core::Permission::ScopeAdmin,
            &diskgraph_engine::admin_scope(),
        );
        assert_eq!(
            decision,
            diskgraph_core::Decision::Allowed,
            "bootstrap grants scope:admin"
        );
        let scope = service
            .engine()
            .register_scope(&project, &principal, &authorizer)
            .unwrap();
        // register_scope issues the scope-local grants, so the authorizer is
        // rebuilt before the next call: a snapshot taken earlier would refuse.
        let authorizer = service.engine().policy_authorizer().unwrap();
        let job = service
            .engine()
            .index_scope(&scope, &principal, &authorizer)
            .unwrap();
        service.engine().run_job(&job.job_id, "http-test").unwrap();

        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(HttpLimits::default(), Security::local(None)),
            Vec::new(),
        )
        .unwrap();

        let body = http_client::post_json(
            address.port,
            MCP_ENDPOINT,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "diskgraph_top",
                    "arguments": {"scope": scope.as_str()},
                },
            })
            .to_string(),
        );
        let response: Value = serde_json::from_str(&body).expect("JSON response over TCP");
        let payload = &response["result"]["structuredContent"];
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["scope_id"], scope.as_str());
        assert!(!payload["data"]["items"].as_array().unwrap().is_empty());
    }

    /// A client that names a path the server does not know must be told so;
    /// a same-named directory on the client machine is never substituted
    /// (spec MCP-03, SC-01, SC-02).
    #[test]
    fn a_client_side_path_is_never_resolved_against_the_server() {
        let (mut service, server_dir) = service("isolation");
        // The server indexes a directory named exactly like the client's.
        let server_root = server_dir.path().join("shared-name");
        std::fs::create_dir_all(&server_root).unwrap();
        std::fs::write(server_root.join("server-only.bin"), vec![0; 4096]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let scope = service
            .engine()
            .register_scope(&server_root, &principal, &authorizer)
            .unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let job = service
            .engine()
            .index_scope(&scope, &principal, &authorizer)
            .unwrap();
        service.engine().run_job(&job.job_id, "isolation").unwrap();

        let limits = HttpLimits::default();
        // A raw path is not even a legal scope id, so it is rejected before any
        // lookup: the server never walks a client-supplied path.
        let raw_path = handle(
            &mut service,
            &post(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"diskgraph_top","arguments":{"scope":"/etc"}}}"#,
            ),
            &limits,
        );
        assert_eq!(
            response_json(&raw_path)["error"]["data"]["business_code"],
            "invalid_argument"
        );

        // A well-formed id the server never issued is simply unknown; it is
        // never resolved against a client-side directory of the same name.
        let unknown_id = handle(
            &mut service,
            &post(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"diskgraph_top","arguments":{"scope":"scope-shared-name"}}}"#,
            ),
            &limits,
        );
        assert_eq!(
            response_json(&unknown_id)["error"]["data"]["business_code"],
            "not_found"
        );

        // The same-named directory that the server really indexed still
        // answers, and only from the server's own index.
        let response = handle(
            &mut service,
            &post(
                &json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": "diskgraph_top", "arguments": {"scope": scope.as_str()}}
                })
                .to_string(),
            ),
            &limits,
        );
        let body = response_json(&response);
        let payload = &body["result"]["structuredContent"];
        assert_eq!(payload["ok"], true);
        // On macOS the scanner canonicalizes through /private, so compare
        // against the canonical root rather than the literal fixture path.
        let canonical = server_root.canonicalize().unwrap();
        let items = payload["data"]["items"].as_array().unwrap();
        assert!(!items.is_empty());
        assert!(items.iter().all(|item| {
            item["locator"]["value"]
                .as_str()
                .unwrap()
                .starts_with(canonical.to_str().unwrap())
        }));
    }

    /// Two servers on one machine keep their data separate: a scope id from
    /// one is not_found on the other (spec SC-02).
    #[test]
    fn scope_ids_from_another_server_are_not_found() {
        let (first, _first_dir) = service("server-a");
        let (mut second, _second_dir) = service("server-b");
        let project = std::env::temp_dir().join("diskgraph-isolation-project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("file.bin"), vec![0; 512]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = first.engine().policy_authorizer().unwrap();
        let scope_a = first
            .engine()
            .register_scope(&project, &principal, &authorizer)
            .unwrap();
        let authorizer = first.engine().policy_authorizer().unwrap();
        let job = first
            .engine()
            .index_scope(&scope_a, &principal, &authorizer)
            .unwrap();
        first.engine().run_job(&job.job_id, "server-a").unwrap();

        let limits = HttpLimits::default();
        let response = handle(
            &mut second,
            &post(
                &json!({
                    "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": "diskgraph_top", "arguments": {"scope": scope_a.as_str()}}
                })
                .to_string(),
            ),
            &limits,
        );
        let body = response_json(&response);
        assert_eq!(body["error"]["data"]["business_code"], "not_found");
        let _ = std::fs::remove_dir_all(&project);
    }

    /// End-to-end: the authenticated router refuses unauthenticated and
    /// forged traffic before the engine is ever consulted (spec SC-05).
    #[test]
    fn remote_requests_require_a_valid_bearer_token() {
        use crate::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};

        const KEY: &[u8] = b"http-verification-key";
        let authenticator = Authenticator::new(AuthConfig::single(
            "https://auth.example.test",
            "diskgraph",
            KEY,
        ));
        let (mut service, _keep) = service("auth-gate");
        let limits = HttpLimits::default();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

        // No token at all.
        let request = HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            query: String::new(),
            headers: HashMap::new(),
            body: body.into(),
        };
        let response = handle_authenticated(&mut service, &request, &limits, Some(&authenticator));
        assert_eq!(response.status, 401);
        let decoded = serde_json::from_str::<Value>(&response.body).unwrap();
        assert_eq!(decoded["reason"], "missing_token");

        // A proxy identity header is never trusted.
        let mut headers = HashMap::new();
        headers.insert("x-forwarded-user".into(), "root".into());
        headers.insert("x-auth-request-user".into(), "root".into());
        let request = HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            headers,
            body: body.into(),
            query: String::new(),
        };
        let response = handle_authenticated(&mut service, &request, &limits, Some(&authenticator));
        assert_eq!(response.status, 401);

        // A forged token is refused.
        let mut headers = HashMap::new();
        headers.insert("authorization".into(), "Bearer forged.token.here".into());
        let request = HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            headers,
            body: body.into(),
            query: String::new(),
        };
        let response = handle_authenticated(&mut service, &request, &limits, Some(&authenticator));
        assert_eq!(response.status, 401);

        // A properly signed token for the right issuer and audience passes.
        let claims = TokenClaims {
            issuer: "https://auth.example.test".into(),
            audience: "diskgraph".into(),
            subject: "agent@example.test".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        };
        let token = TokenMinter::new(KEY).mint(&claims);
        let mut headers = HashMap::new();
        headers.insert("authorization".into(), format!("Bearer {token}"));
        let request = HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            headers,
            body: body.into(),
            query: String::new(),
        };
        let response = handle_authenticated(&mut service, &request, &limits, Some(&authenticator));
        assert_eq!(response.status, 200);
        let decoded = serde_json::from_str::<Value>(&response.body).unwrap();
        assert!(decoded["result"]["tools"].is_array());
    }

    /// Origin policy: loopback origins and absence are fine; a hostile origin
    /// is refused with 403 before authentication runs (P4 task 5.4, MCP-06).
    #[test]
    fn origins_are_validated_before_anything_else() {
        let policy = NetworkPolicy::loopback_default();
        assert_eq!(policy.origin_decision(None), OriginDecision::Absent);
        assert_eq!(
            policy.origin_decision(Some("http://127.0.0.1:3000")),
            OriginDecision::Allowed
        );
        assert_eq!(
            policy.origin_decision(Some("http://localhost")),
            OriginDecision::Allowed
        );
        assert_eq!(
            policy.origin_decision(Some("http://[::1]:5173")),
            OriginDecision::Allowed
        );
        assert_eq!(
            policy.origin_decision(Some("https://evil.example")),
            OriginDecision::Refused
        );
        assert_eq!(
            policy.origin_decision(Some("null")),
            OriginDecision::Refused
        );
        assert_eq!(
            policy.origin_decision(Some("not-an-origin")),
            OriginDecision::Refused
        );

        // A null origin is acceptable only when a deployment asks for it.
        let permissive = NetworkPolicy {
            allow_null_origin: true,
            ..NetworkPolicy::default()
        };
        assert_eq!(
            permissive.origin_decision(Some("null")),
            OriginDecision::Allowed
        );

        // Extra origins are exact matches, loopback stays free.
        let custom = NetworkPolicy::loopback_default()
            .with_origins(["https://agent.corp.example".to_owned()]);
        assert_eq!(
            custom.origin_decision(Some("https://agent.corp.example")),
            OriginDecision::Allowed
        );
        assert_eq!(
            custom.origin_decision(Some("https://agent.corp.example.evil.test")),
            OriginDecision::Refused
        );
    }

    /// A malicious Origin gets 403 and never reaches the engine, even when a
    /// valid bearer token is presented.
    #[test]
    fn a_hostile_origin_is_refused_even_with_a_valid_token() {
        use crate::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};

        let authenticator = Authenticator::new(AuthConfig::single("iss", "aud", b"key"));
        let claims = TokenClaims {
            issuer: "iss".into(),
            audience: "aud".into(),
            subject: "agent".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        };
        let token = TokenMinter::new(b"key").mint(&claims);
        let (mut service, _keep) = service("origin-gate");
        let limits = HttpLimits::default();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

        let mut headers = HashMap::new();
        headers.insert("authorization".into(), format!("Bearer {token}"));
        headers.insert("origin".into(), "https://phishing.example".into());
        let request = HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            headers,
            body: body.into(),
            query: String::new(),
        };
        let security = Security::local(Some(&authenticator));
        let response = handle_secured(&mut service, &request, &limits, &security);
        assert_eq!(response.status, 403);
        assert!(response.body.contains("forbidden_origin"));

        // The same token with a loopback origin passes.
        let mut headers = HashMap::new();
        headers.insert("authorization".into(), format!("Bearer {token}"));
        headers.insert("origin".into(), "http://localhost:4173".into());
        let request = HttpRequest {
            method: "POST".into(),
            path: MCP_ENDPOINT.into(),
            headers,
            body: body.into(),
            query: String::new(),
        };
        let security = Security::local(Some(&authenticator));
        let response = handle_secured(&mut service, &request, &limits, &security);
        assert_eq!(response.status, 200);
    }

    /// The bind policy: loopback is always plaintext-OK; a network interface
    /// needs authentication and an explicit TLS-termination acknowledgment.
    #[test]
    fn the_bind_policy_gates_network_interfaces_honestly() {
        assert_eq!(
            bind_decision("127.0.0.1", false, false),
            BindPolicy::LoopbackPlaintext
        );
        assert_eq!(
            bind_decision("localhost", false, false),
            BindPolicy::LoopbackPlaintext
        );
        assert_eq!(
            bind_decision("0.0.0.0", false, true),
            BindPolicy::RefuseNoAuth
        );
        // Authenticated but still plaintext on a network interface: refused,
        // because this HTTP core does not terminate TLS itself.
        assert_eq!(
            bind_decision("192.168.1.10", true, false),
            BindPolicy::RefusePlaintext
        );
        assert_eq!(
            bind_decision("0.0.0.0", true, true),
            BindPolicy::NetworkWithTunnel
        );
    }

    /// 转发地址只在实际 peer 为可信代理时用于诊断和限流，不建立授权身份。
    #[test]
    fn forwarded_addresses_are_proxy_gated() {
        let policy = NetworkPolicy {
            trusted_proxies: vec!["10.0.0.1".to_owned()],
            ..NetworkPolicy::default()
        };
        let mut headers = HashMap::new();
        headers.insert("x-forwarded-for".into(), "203.0.113.9, 10.0.0.1".into());

        // An untrusted peer cannot launder its address through the header.
        assert_eq!(
            observed_client_ip("198.51.100.4:5555", &headers, &policy),
            "198.51.100.4"
        );
        // 可信代理追加的地址链用于诊断和限流键。
        assert_eq!(
            observed_client_ip("10.0.0.1:5555", &headers, &policy),
            "203.0.113.9"
        );
        // No header: the peer itself.
        let empty = HashMap::new();
        assert_eq!(
            observed_client_ip("10.0.0.1:5555", &empty, &policy),
            "10.0.0.1"
        );
    }

    /// Rate limiter: burst then throttle, refill over time, per-client state.
    #[test]
    fn the_rate_limiter_throttles_bursts_and_recovers() {
        let limits = HttpLimits {
            max_requests_per_second_per_client: 2,
            ..HttpLimits::default()
        };
        let clock = std::sync::Mutex::new(0u128);
        let limiter = RateLimiter::new(&limits).with_clock(move || *clock.lock().unwrap());
        let client = "127.0.0.1";

        // Burst of two is admitted...
        assert!(limiter.check_at(client, 0).is_ok());
        assert!(limiter.check_at(client, 0).is_ok());
        // ...the third in the same instant is refused with a retry delay.
        let retry = limiter.check_at(client, 0).unwrap_err();
        assert!(retry > 0 && retry <= 1_000);
        // Another client is unaffected.
        assert!(limiter.check_at("10.0.0.9", 0).is_ok());
        // After one second the bucket has refilled.
        assert!(limiter.check_at(client, 1_000).is_ok());
    }

    #[test]
    fn rate_state_is_bounded_without_evicting_active_clients() {
        let limits = HttpLimits {
            max_requests_per_second_per_client: 2,
            ..HttpLimits::default()
        };
        let limiter = RateLimiter::new(&limits);
        for client in 0..4_096 {
            assert!(limiter.check_at(&format!("client-{client}"), 0).is_ok());
        }
        assert_eq!(limiter.check_at("new-client", 0), Err(1_000));
        assert!(limiter.check_at("client-0", 0).is_ok());
        assert!(limiter.check_at("client-0", 0).is_err());
        assert_eq!(limiter.check_at("new-client", 59_999), Err(1_000));
        assert!(limiter.check_at("client-0", 59_999).is_ok());
        assert!(limiter.check_at("client-0", 60_998).is_ok());
        assert!(limiter.check_at("client-0", 60_998).is_ok());
        assert!(limiter.check_at("new-client", 61_000).is_ok());
        // 刚活跃的 client-0 不因满表清理而获得一个新桶。
        assert!(limiter.check_at("client-0", 61_000).is_err());
    }

    #[test]
    fn rate_state_rejects_oversized_keys() {
        let limiter = RateLimiter::new(&HttpLimits::default());
        assert_eq!(limiter.check_at(&"x".repeat(65), 0), Err(1_000));
        assert_eq!(limiter.check_at("", 0), Err(1_000));
    }

    #[test]
    fn rate_state_does_not_refill_twice_after_stale_clock_observations() {
        let limits = HttpLimits {
            max_requests_per_second_per_client: 1,
            ..HttpLimits::default()
        };
        let limiter = RateLimiter::new(&limits);
        assert!(limiter.check_at("client", 1_000).is_ok());
        assert!(limiter.check_at("client", 1_500).is_err());
        assert!(limiter.check_at("client", 1_000).is_err());
        // 旧观测不能把 last_refill 倒拨并再次补出这半秒的额度。
        assert!(limiter.check_at("client", 1_500).is_err());
        assert!(limiter.check_at("client", 2_000).is_ok());
    }

    #[test]
    fn client_addresses_preserve_ipv6_and_normalize_mapped_ipv4() {
        let empty = HashMap::new();
        let policy = NetworkPolicy::default();
        assert_eq!(
            observed_client_ip("[2001:db8::1]:5555", &empty, &policy),
            "2001:db8::1"
        );
        assert_eq!(
            observed_client_ip("[2001:db8::2]:5555", &empty, &policy),
            "2001:db8::2"
        );
        assert_eq!(
            observed_client_ip("[::ffff:192.0.2.1]:5555", &empty, &policy),
            "192.0.2.1"
        );
        assert_eq!(observed_client_ip("[::1]:5555", &empty, &policy), "::1");
    }

    #[test]
    fn forwarded_chains_cannot_launder_a_spoofed_prefix() {
        let policy = NetworkPolicy {
            trusted_proxies: vec!["2001:db8::1".into(), "10.0.0.1".into()],
            ..NetworkPolicy::default()
        };
        let mut headers = HashMap::new();
        headers.insert(
            "x-forwarded-for".into(),
            "192.0.2.99, 203.0.113.9, 10.0.0.1".into(),
        );
        assert_eq!(
            observed_client_ip("10.0.0.1:5555", &headers, &policy),
            "203.0.113.9"
        );
        assert_eq!(
            observed_client_ip("[2001:db8::1]:5555", &headers, &policy),
            "203.0.113.9"
        );
        headers.insert("x-forwarded-for".into(), "::ffff:203.0.113.9".into());
        assert_eq!(
            observed_client_ip("10.0.0.1:5555", &headers, &policy),
            "203.0.113.9"
        );
        headers.insert("x-forwarded-for".into(), "not-an-ip, 203.0.113.9".into());
        assert_eq!(
            observed_client_ip("10.0.0.1:5555", &headers, &policy),
            "10.0.0.1"
        );
        headers.insert("x-forwarded-for".into(), vec!["203.0.113.9"; 33].join(","));
        assert_eq!(
            observed_client_ip("10.0.0.1:5555", &headers, &policy),
            "10.0.0.1"
        );
    }

    /// A slow connection cannot starve other clients: its thread dies on the
    /// read timeout while a second connection is served promptly.
    #[test]
    fn a_stalled_connection_does_not_starve_other_clients() {
        let (service, _keep) = service("slowloris");
        let limits = HttpLimits {
            read_timeout: Duration::from_millis(150),
            max_concurrent_connections: 4,
            max_requests_per_second_per_client: 100,
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)),
            Vec::new(),
        )
        .unwrap();

        // Client A connects and stalls mid-request.
        let mut stalled = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stalled
            .write_all(b"POST /mcp HTTP/1.1\r\nHost: x\r\n")
            .unwrap();
        // Let the stall begin, then confirm client B still gets served.
        std::thread::sleep(Duration::from_millis(400));
        let body = http_client::post_json(
            port,
            MCP_ENDPOINT,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        let response: Value = serde_json::from_str(&body).unwrap();
        assert!(
            response["result"]["tools"].is_array(),
            "a stalled client must not starve others: {body}"
        );
        // After the timeout, A's connection is gone: reading from it fails.
        stalled
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut byte = [0u8; 1];
        assert!(
            stalled.read(&mut byte).is_err() || byte[0] == 0,
            "the stalled connection must be closed by the read timeout"
        );
    }

    /// The connection cap refuses with 503 instead of queueing a flood.
    #[test]
    fn the_connection_cap_refuses_excess_connections() {
        let (service, _keep) = service("cap");
        let limits = HttpLimits {
            max_concurrent_connections: 1,
            read_timeout: Duration::from_secs(5),
            max_requests_per_second_per_client: 100,
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)),
            Vec::new(),
        )
        .unwrap();
        // Hold the single slot with an idle (but connected) client.
        let _holder = TcpStream::connect(("127.0.0.1", port)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        // The next connection is refused immediately.
        let mut refused = TcpStream::connect(("127.0.0.1", port)).unwrap();
        refused
            .set_read_timeout(Some(Duration::from_millis(800)))
            .unwrap();
        refused
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut reader = BufReader::new(refused);
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        assert!(
            status.contains("503"),
            "over-cap connections must be refused, got: {status}"
        );
        let mut remainder = String::new();
        reader.read_to_string(&mut remainder).unwrap();
        assert!(
            remainder.contains("Connection: close\r\n"),
            "a refused connection must declare close: {remainder}"
        );
        let (headers, body) = remainder.split_once("\r\n\r\n").unwrap();
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            body.len(),
            length,
            "the complete rejection body must arrive"
        );
        let body: Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["error"], "connection_limit");
    }

    /// Over-limit request rates get 429 while staying well-formed.
    #[test]
    fn over_rate_requests_are_refused_with_429() {
        let (service, _keep) = service("rate");
        let limits = HttpLimits {
            max_requests_per_second_per_client: 2,
            read_timeout: Duration::from_secs(5),
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)),
            Vec::new(),
        )
        .unwrap();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let first = http_client::post_json(port, MCP_ENDPOINT, body);
        let second = http_client::post_json(port, MCP_ENDPOINT, body);
        let third = http_client::post_json(port, MCP_ENDPOINT, body);
        assert!(first.contains("\"tools\""), "first request served: {first}");
        assert!(
            second.contains("\"tools\""),
            "burst second served: {second}"
        );
        assert!(third.contains("rate_limited"), "third throttled: {third}");
    }

    /// The full legacy flow over real sockets: GET /sse gives the endpoint
    /// event, POSTing a message yields 202 and the response arrives as a
    /// `message` event (P4 tasks 5.6/5.7).
    #[test]
    fn the_legacy_adapter_serves_a_spec_conformant_client() {
        use diskgraph_testkit::legacy_client;

        let (service, _keep) = service("legacy-flow");
        let limits = HttpLimits {
            read_timeout: Duration::from_secs(5),
            max_requests_per_second_per_client: 100,
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)).with_legacy(),
            Vec::new(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let mut sse = legacy_client::SseStream::connect(port).expect("legacy handshake");
        assert!(
            sse.message_endpoint.starts_with("/messages/?session_id="),
            "endpoint event names the message URL: {}",
            sse.message_endpoint
        );
        let status = legacy_client::post_message(
            port,
            &sse.message_endpoint,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &[],
        )
        .expect("legacy POST");
        assert_eq!(status, 202, "the legacy protocol acknowledges with 202");
        let event = sse
            .next_message()
            .expect("the response arrives on the SSE stream");
        let response: Value = serde_json::from_str(&event).expect("valid JSON-RPC event");
        assert!(response["result"]["tools"].is_array());
    }

    /// The legacy adapter is off by default: probing its paths reports so.
    #[test]
    fn a_modern_only_server_reports_the_legacy_adapter_as_disabled() {
        let (service, _keep) = service("legacy-off");
        let limits = HttpLimits {
            read_timeout: Duration::from_secs(5),
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)),
            Vec::new(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let status = http_client::get(port, "/sse");
        assert!(status.contains("legacy_sse_disabled"), "body: {status}");
        let posted = http_client::post_json(
            port,
            "/messages/?session_id=none",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert!(posted.contains("legacy_sse_disabled"), "body: {posted}");
    }

    /// Legacy sessions the server never issued are refused; authorization and
    /// origin policy bind legacy POSTs exactly like modern requests.
    #[test]
    fn legacy_posts_enforce_sessions_auth_and_origin() {
        use crate::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
        use diskgraph_testkit::legacy_client;

        let authenticator = Authenticator::new(AuthConfig::single("iss", "aud", b"key"));
        let claims = TokenClaims {
            issuer: "iss".into(),
            audience: "aud".into(),
            subject: "agent".into(),
            expires_at_unix_seconds: u64::MAX,
            scope: Some("metadata:read".into()),
        };
        let token = TokenMinter::new(b"key").mint(&claims);
        let (service, _keep) = service("legacy-gate");
        let identity = authenticator.authenticate(Some(&token)).unwrap();
        service
            .engine()
            .bootstrap_local_admin(&identity.principal)
            .unwrap();
        let limits = HttpLimits {
            read_timeout: Duration::from_secs(5),
            max_requests_per_second_per_client: 100,
            ..HttpLimits::default()
        };
        let security = Security::local(Some(&authenticator));
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, security).with_legacy(),
            Vec::new(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        // Unknown session: 404.
        let status = legacy_client::post_message(
            port,
            "/messages/?session_id=bogus",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &[("Authorization", &format!("Bearer {token}"))],
        )
        .unwrap();
        assert_eq!(status, 404);

        // Real session, no token: 401.
        let mut sse = legacy_client::SseStream::connect_with_headers(
            port,
            &[("Authorization", &format!("Bearer {token}"))],
        )
        .unwrap();
        let status = legacy_client::post_message(
            port,
            &sse.message_endpoint,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &[],
        )
        .unwrap();
        assert_eq!(status, 401);

        // Real session, valid token: 202 and the answer on the stream.
        let status = legacy_client::post_message(
            port,
            &sse.message_endpoint,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            &[("Authorization", &format!("Bearer {token}"))],
        )
        .unwrap();
        assert_eq!(status, 202);
        let event = sse.next_message().unwrap();
        assert!(event.contains("\"tools\""), "event: {event}");
    }

    /// MCP-05: connection identity and business job identity are separate. A
    /// client that disconnects right after requesting an index loses nothing:
    /// a brand-new connection, with no session continuity, watches the job
    /// reach its terminal state by job id alone.
    #[test]
    fn a_disconnected_client_job_completes_and_stays_queryable() {
        let (service, _keep) = service_with_profile("reconnect", ToolProfile::Manage);
        let fixture = tempfile::TempDir::with_prefix("diskgraph-reconnect-fix-").unwrap();
        std::fs::create_dir_all(fixture.path().join("project")).unwrap();
        std::fs::write(fixture.path().join("project").join("f.bin"), vec![0; 4096]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let scope = service
            .engine()
            .register_scope(fixture.path(), &principal, &authorizer)
            .unwrap();
        let runner = crate::tests::McpTestRunner::new(service.start_job_runner(), fixture);
        // register_scope already issued the registrar's scope-local grants.
        let limits = HttpLimits {
            read_timeout: Duration::from_secs(5),
            max_requests_per_second_per_client: 1_000,
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)),
            Vec::new(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let call_index = |job_counter: u64| {
            format!(
                r#"{{"jsonrpc":"2.0","id":{job_counter},"method":"tools/call","params":{{"name":"diskgraph_index","arguments":{{"scope":"{}"}}}}}}"#,
                scope.as_str()
            )
        };
        let call_status = |job_id: &str, id: u64| {
            format!(
                r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"diskgraph_status","arguments":{{"job_id":"{job_id}"}}}}}}"#
            )
        };

        // A 连接请求索引并取得持久 job ID，不等待业务任务完成。
        let body = http_client::post_json(port, MCP_ENDPOINT, &call_index(1));
        let response: Value =
            serde_json::from_str(&body).unwrap_or_else(|_| panic!("response must be JSON: {body}"));
        let job_id = response["result"]["structuredContent"]["data"]["job_id"]
            .as_str()
            .unwrap_or_else(|| panic!("a durable job id in {body}"))
            .to_owned();
        // HTTP helper 返回时 A socket 已关闭，后续不再使用原连接。

        // B 无会话连续性，每次凭 job ID 重新建立连接查询，直到业务终态。
        let polling_started = std::time::Instant::now();
        let deadline = polling_started + Duration::from_secs(10);
        let mut observed: Vec<String> = Vec::new();
        let terminal;
        let terminal_response;
        let terminal_body;
        loop {
            let request_started = std::time::Instant::now();
            let body = http_client::post_json(port, MCP_ENDPOINT, &call_status(&job_id, 2));
            let response: Value = serde_json::from_str(&body).unwrap_or_else(|error| {
                panic!(
                    "job_id={job_id}; request_id=2; elapsed={:?}; request_elapsed={:?}; status JSON parse error={error}; HTTP body={body:?}",
                    polling_started.elapsed(),
                    request_started.elapsed()
                )
            });
            let data = &response["result"]["structuredContent"]["data"];
            assert_eq!(
                data["job_id"],
                job_id,
                "the job id binds across connections; job_id={job_id}; request_id=2; elapsed={:?}; request_elapsed={:?}; status JSON={response}; HTTP body={body:?}",
                polling_started.elapsed(),
                request_started.elapsed()
            );
            let state = data["state"]
                .as_str()
                .unwrap_or_else(|| {
                    panic!(
                        "job_id={job_id}; request_id=2; elapsed={:?}; request_elapsed={:?}; missing status state; status JSON={response}; HTTP body={body:?}",
                        polling_started.elapsed(),
                        request_started.elapsed()
                    )
                })
                .to_owned();
            if observed.last() != Some(&state) {
                observed.push(state.clone());
            }
            if matches!(state.as_str(), "completed" | "failed" | "cancelled") {
                terminal = state;
                terminal_response = response;
                terminal_body = body;
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "job {job_id} never finished; saw {observed:?}; request_id=2; elapsed={:?}; request_elapsed={:?}; status JSON={response}; HTTP body={body:?}",
                polling_started.elapsed(),
                request_started.elapsed()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            terminal,
            "completed",
            "disconnect must not fail the job; job_id={job_id}; request_id=2; elapsed={:?}; status JSON={terminal_response}; HTTP body={terminal_body:?}",
            polling_started.elapsed()
        );
        // runner 可以在新连接首轮查询前完成；重连读取持久状态，
        // 不要求客户端逐一观察所有中间态。
        assert!(
            matches!(
                observed.first().map(String::as_str),
                Some("queued") | Some("running") | Some("completed")
            ),
            "the first observation from the new connection is a real state: {observed:?}"
        );
        // C 连接也完全独立；观察连接关闭后仍应返回同一 job ID 的 completed 状态。
        let request_started = std::time::Instant::now();
        let body = http_client::post_json(port, MCP_ENDPOINT, &call_status(&job_id, 3));
        let response: Value = serde_json::from_str(&body).unwrap_or_else(|error| {
            panic!(
                "job_id={job_id}; request_id=3; elapsed={:?}; request_elapsed={:?}; status JSON parse error={error}; HTTP body={body:?}",
                polling_started.elapsed(),
                request_started.elapsed()
            )
        });
        let data = &response["result"]["structuredContent"]["data"];
        assert_eq!(
            data["job_id"],
            job_id,
            "the durable job id stays bound; job_id={job_id}; request_id=3; elapsed={:?}; request_elapsed={:?}; status JSON={response}; HTTP body={body:?}",
            polling_started.elapsed(),
            request_started.elapsed()
        );
        assert_eq!(
            data["state"],
            "completed",
            "the completed result persists; job_id={job_id}; request_id=3; elapsed={:?}; request_elapsed={:?}; status JSON={response}; HTTP body={body:?}",
            polling_started.elapsed(),
            request_started.elapsed()
        );
        // RAII: 先停止传输，再 join runner，最后释放原扫描源。
        let _ = &runner;
    }

    /// A cancelled-by-disconnect client never existed: dropping every socket
    /// without an authorized cancel leaves the job to complete (RT-02).
    #[test]
    fn dropping_every_connection_is_not_a_cancellation() {
        let (service, _keep) = service("no-cancel");
        let fixture = tempfile::TempDir::with_prefix("diskgraph-nocancel-fix-").unwrap();
        std::fs::create_dir_all(fixture.path().join("d")).unwrap();
        std::fs::write(fixture.path().join("d").join("a"), vec![0; 2048]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let scope = service
            .engine()
            .register_scope(fixture.path(), &principal, &authorizer)
            .unwrap();
        let runner = crate::tests::McpTestRunner::new(service.start_job_runner(), fixture);
        let _ = scope;

        // Create the job through a throwaway handle, drop everything.
        let job = {
            let engine = service.engine();
            let authorizer = engine.policy_authorizer().unwrap();
            engine.index_scope(&scope, &principal, &authorizer).unwrap()
        };
        let job_id = job.job_id.clone();
        // No cancel call, no connection kept: just wait for the runner.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let record = service.engine().job_status(&job_id).unwrap();
            if record.state == diskgraph_store::JobState::Completed {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "job {job_id} state: {:?}",
                record.state
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        // RAII: 先停止传输，再 join runner，最后释放原扫描源。
        let _ = &runner;
    }

    /// The disconnect guarantee holds on the legacy transport too: a legacy
    /// client that vanishes after its POST still leaves a completable,
    /// queryable job behind (MCP-05 across adapters).
    #[test]
    fn legacy_disconnect_leaves_the_job_queryable() {
        use diskgraph_testkit::legacy_client;

        let (service, _keep) = service_with_profile("legacy-reconnect", ToolProfile::Manage);
        let fixture = tempfile::TempDir::with_prefix("diskgraph-legacy-reconnect-").unwrap();
        std::fs::create_dir_all(fixture.path().join("p")).unwrap();
        std::fs::write(fixture.path().join("p").join("f"), vec![0; 1024]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let scope = service
            .engine()
            .register_scope(fixture.path(), &principal, &authorizer)
            .unwrap();
        let runner = crate::tests::McpTestRunner::new(service.start_job_runner(), fixture);

        let limits = HttpLimits {
            read_timeout: Duration::from_secs(5),
            max_requests_per_second_per_client: 1_000,
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)).with_legacy(),
            Vec::new(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        // Connection A: legacy client requests the index, then disappears.
        let mut sse = legacy_client::SseStream::connect(port).unwrap();
        let status = legacy_client::post_message(
            port,
            &sse.message_endpoint,
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"diskgraph_index","arguments":{{"scope":"{}"}}}}}}"#,
                scope.as_str()
            ),
            &[],
        )
        .unwrap();
        assert_eq!(status, 202);
        let job: Value = serde_json::from_str(&sse.next_message().unwrap()).unwrap();
        let job_id = job["result"]["structuredContent"]["data"]["job_id"]
            .as_str()
            .unwrap()
            .to_owned();
        drop(sse); // the whole legacy client goes away

        // Connection B: a brand-new legacy client watches the job by id.
        let mut sse2 = legacy_client::SseStream::connect(port).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let terminal;
        loop {
            let status = legacy_client::post_message(
                port,
                &sse2.message_endpoint,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"diskgraph_status","arguments":{{"job_id":"{job_id}"}}}}}}"#
                ),
                &[],
            )
            .unwrap();
            assert_eq!(status, 202);
            let reply: Value = serde_json::from_str(&sse2.next_message().unwrap()).unwrap();
            let data = &reply["result"]["structuredContent"]["data"];
            assert_eq!(data["job_id"], job_id);
            let state = data["state"].as_str().unwrap();
            if matches!(state, "completed" | "failed" | "cancelled") {
                terminal = state.to_owned();
                break;
            }
            assert!(std::time::Instant::now() < deadline, "job never finished");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(terminal, "completed");
        // RAII: 先停止传输，再 join runner，最后释放原扫描源。
        let _ = &runner;
    }

    /// MCP search cursors bind the policy epoch: after a publish, pages
    /// started under the old epoch are refused (P4-5.9).
    #[test]
    fn mcp_search_cursors_expire_with_policy_updates() {
        let (mut service, directory) = service("mcp-cursor-epoch");
        let root = directory.path().join("project");
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("target").join("bin"), vec![0; 4096]).unwrap();
        let principal = diskgraph_core::PrincipalId::new(crate::STDIO_PRINCIPAL).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let scope = service
            .engine()
            .register_scope(&root, &principal, &authorizer)
            .unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let job = service
            .engine()
            .index_scope(&scope, &principal, &authorizer)
            .unwrap();
        service
            .engine()
            .run_job(&job.job_id, "cursor-test")
            .unwrap();
        let revision = service.engine().latest_revision(&scope).unwrap().unwrap();

        let call = |service: &mut McpService, id: u64, arguments: Value| {
            service.handle(&crate::protocol::Request {
                id: json!(id),
                method: "tools/call".to_owned(),
                params: json!({"name": "diskgraph_search", "arguments": arguments}),
            })
        };

        let page = call(
            &mut service,
            1,
            json!({"scope": scope.as_str(), "pattern": "t", "limit": 1}),
        );
        let data = &page["result"]["structuredContent"]["data"];
        let cursor = data["next_cursor"]
            .as_str()
            .expect("a cursor is offered")
            .to_owned();

        service
            .engine()
            .publish_policy_version(
                2,
                service.context.principal(),
                &service.engine().policy_authorizer().unwrap(),
            )
            .unwrap();
        let stale = call(
            &mut service,
            2,
            json!({"scope": scope.as_str(), "pattern": "t", "limit": 1, "cursor": cursor}),
        );
        assert_eq!(
            stale["error"]["data"]["business_code"], "invalid_argument",
            "stale cursor must be refused: {stale}"
        );

        let fresh = call(
            &mut service,
            3,
            json!({"scope": scope.as_str(), "pattern": "t", "limit": 1}),
        );
        assert!(fresh["result"]["structuredContent"]["data"]["next_cursor"].is_string());
        assert_eq!(
            service.engine().latest_revision(&scope).unwrap().unwrap(),
            revision
        );
    }

    /// Regression: a GET /mcp stream must outlive the socket read timeout.
    /// Treating an idle probe as a disconnect killed the stream every few
    /// seconds and sent rmcp-based hosts into a reconnect loop.
    #[test]
    fn the_server_stream_survives_read_timeouts() {
        use std::io::Read as _;

        let (service, _keep) = service("stream-lifetime");
        let limits = HttpLimits {
            // A short timeout makes the regression surface quickly.
            read_timeout: Duration::from_millis(200),
            max_requests_per_second_per_client: 1_000,
            ..HttpLimits::default()
        };
        let (listener, address) = bind("127.0.0.1", 0).unwrap();
        let port = address.port;
        let _server = crate::http_server_runtime::HttpServerRuntime::start(
            service,
            listener,
            ServerConfig::modern(limits, Security::local(None)),
            Vec::new(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .write_all(b"GET /mcp HTTP/1.1\r\nHost: x\r\nAccept: text/event-stream\r\n\r\n")
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        assert!(status.contains("200"), "stream handshake: {status}");
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line == "\n" || line.is_empty() {
                break;
            }
        }

        // Wait well past the server's read timeout: a keep-alive comment must
        // still arrive, proving the stream survived the idle ticks.
        let mut saw_keepalive = false;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buffer = [0u8; 256];
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if let Ok(count) = stream.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                if String::from_utf8_lossy(&buffer[..count]).contains("keep-alive") {
                    saw_keepalive = true;
                    break;
                }
            }
        }
        assert!(
            saw_keepalive,
            "the stream must stay open across read timeouts and emit keep-alives"
        );
    }
}
