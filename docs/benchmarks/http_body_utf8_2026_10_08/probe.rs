#![allow(dead_code)]
use std::collections::HashMap;
use std::io::{BufRead,BufReader,Read,ErrorKind};
use std::net::TcpStream;
use std::time::{Instant,Duration};
#[path = "/Users/wandl/workspaces/workspace-loong10k/diskgraph/crates/diskgraph-mcp/src/http_request.rs"] mod http_request;
#[path = "/Users/wandl/workspaces/workspace-loong10k/diskgraph/crates/diskgraph-mcp/src/http_limits.rs"] mod http_limits;
#[path = "/Users/wandl/workspaces/workspace-loong10k/diskgraph/crates/diskgraph-mcp/src/http_shutdown_state.rs"] mod http_shutdown_state;
#[path = "/Users/wandl/workspaces/workspace-loong10k/diskgraph/crates/diskgraph-mcp/src/http_request_syntax_tests.rs"] mod http_request_syntax_tests;
pub use http_request::HttpRequest;
pub use http_limits::HttpLimits;
mod http { pub use crate::{HttpRequest,HttpLimits,read_request}; }
const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_HEADER_COUNT: usize = 100;
pub fn read_request(
    reader: &mut BufReader<TcpStream>,
    limits: &HttpLimits,
) -> std::io::Result<Option<HttpRequest>> {
    read_request_with_shutdown(reader, limits, None)
}

fn read_request_with_shutdown(
    reader: &mut BufReader<TcpStream>,
    limits: &HttpLimits,
    shutdown: Option<&crate::http_shutdown_state::HttpShutdownState>,
) -> std::io::Result<Option<HttpRequest>> {
    let deadline = Instant::now() + limits.read_timeout;
    let Some(request_line) = read_bounded_line(reader, MAX_REQUEST_LINE_BYTES, deadline, shutdown)?
    else {
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
        let Some(line) = read_bounded_line(reader, remaining, deadline, shutdown)? else {
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
        let line = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(&line);
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidData, "malformed HTTP header"))?;
        let name = name.to_ascii_lowercase();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() && byte != b'\t')
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
                    existing.push_str(value.trim_matches([' ', '\t']));
                }
            })
            .or_insert_with(|| value.trim_matches([' ', '\t']).to_owned());
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
        apply_read_deadline(reader, deadline, shutdown)?;
        let count = match reader.read(&mut body[filled..]) {
            Ok(count) => count,
            Err(error) if retry_runtime_read(&error, deadline, shutdown) => continue,
            Err(error) => return Err(normalize_request_timeout(error)),
        };
        if count == 0 {
            return Err(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "incomplete HTTP body",
            ));
        }
        filled += count;
    }
    // 网络 JSON 必须保持原输入语义；非法编码不能替换字符后成为另一个请求。
    let body = String::from_utf8(body)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "HTTP body is not UTF-8"))?;
    Ok(Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    }))
}

/// Reads at most `limit` bytes, including CRLF, without allowing BufRead's
/// unbounded `read_line` allocation or a slow client to renew the deadline.
fn read_bounded_line(
    reader: &mut BufReader<TcpStream>,
    limit: usize,
    deadline: Instant,
    shutdown: Option<&crate::http_shutdown_state::HttpShutdownState>,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        apply_read_deadline(reader, deadline, shutdown)?;
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if retry_runtime_read(&error, deadline, shutdown) => continue,
            Err(error) => return Err(normalize_request_timeout(error)),
        };
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
    shutdown: Option<&crate::http_shutdown_state::HttpShutdownState>,
) -> std::io::Result<()> {
    if shutdown.is_some_and(crate::http_shutdown_state::HttpShutdownState::stopped) {
        return Err(std::io::Error::new(
            ErrorKind::ConnectionAborted,
            "HTTP server stopped",
        ));
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(std::io::Error::new(
            ErrorKind::TimedOut,
            "HTTP request deadline exceeded",
        ));
    }
    // Windows socket shutdown 不单独作为读线程终止证明；短等待复查同一停止状态。
    // 原请求期限只生成一次，分段等待不清空已有行/正文，也不补充剩余时间。
    let wait = if shutdown.is_some() {
        remaining.min(Duration::from_millis(50))
    } else {
        remaining
    };
    reader.get_ref().set_read_timeout(Some(wait))
}

fn retry_runtime_read(
    error: &std::io::Error,
    deadline: Instant,
    shutdown: Option<&crate::http_shutdown_state::HttpShutdownState>,
) -> bool {
    shutdown.is_some()
        && Instant::now() < deadline
        && matches!(
            error.kind(),
            ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
        )
}

fn normalize_request_timeout(error: std::io::Error) -> std::io::Error {
    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
        std::io::Error::new(ErrorKind::TimedOut, "HTTP request deadline exceeded")
    } else {
        error
    }
}

