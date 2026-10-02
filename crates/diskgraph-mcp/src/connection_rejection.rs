//! 超连接上限的有界协议拒绝；不进入认证、工具或业务线程。

use std::io::{ErrorKind, Read};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::http::{HttpResponse, write_closing_response};

const CLOSE_BUDGET: Duration = Duration::from_millis(50);
const MAX_DRAIN_BYTES: usize = 64 * 1024;

/// 发送 503、半关闭写端，在绝对期限和固定字节预算内清理接收端。
/// 来源：DiskGraph 原生 Rust HTTP 接纳控制；无 Java 对应实现。
/// 参数：stream：超上限的 TCP 流；maximum：响应中报告的连接配额。
/// 返回：实际丢弃的输入字节数；预算耗尽直接结束，连接错误向调用者传播。
pub(crate) fn reject_connection(stream: &mut TcpStream, maximum: usize) -> std::io::Result<usize> {
    let deadline = Instant::now() + CLOSE_BUDGET;
    stream.set_write_timeout(Some(CLOSE_BUDGET))?;
    write_closing_response(
        stream,
        &HttpResponse::json(503, json!({"error": "connection_limit", "max": maximum})),
    )?;
    // 先发送 FIN，让正常客户端读到完整响应。立即关闭带未读请求的 socket
    // 会在 Windows/macOS 上触发 RST，覆盖刚发出的 503。
    stream.shutdown(Shutdown::Write)?;
    let mut discarded = 0usize;
    let mut buffer = [0u8; 4096];
    while discarded < MAX_DRAIN_BYTES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        // 每次读取只使用剩余预算；滴流数据不能续期，也不解码或分配正文。
        stream.set_read_timeout(Some(remaining))?;
        let available = buffer.len().min(MAX_DRAIN_BYTES - discarded);
        match stream.read(&mut buffer[..available]) {
            Ok(0) => break,
            Ok(count) => discarded += count,
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                break;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(discarded)
}

#[cfg(test)]
mod tests {
    use super::{MAX_DRAIN_BYTES, reject_connection};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    #[test]
    fn a_silent_rejected_peer_does_not_hold_the_request_timeout() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let worker = std::thread::spawn(move || {
            let started = Instant::now();
            let drained = reject_connection(&mut server, 1).unwrap();
            (drained, started.elapsed())
        });
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
        assert!(response.contains("Connection: close\r\n"));
        // 保持客户端存活且不发送数据，确保 deadline 本身释放清理过程。
        let (drained, elapsed) = worker.join().unwrap();
        assert_eq!(drained, 0);
        assert!(elapsed < Duration::from_millis(500), "elapsed={elapsed:?}");
    }

    #[test]
    fn rejected_body_cleanup_never_reads_beyond_64_kib() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        client
            .write_all(&vec![b'x'; MAX_DRAIN_BYTES + 4096])
            .unwrap();
        let started = Instant::now();
        let drained = reject_connection(&mut server, 1).unwrap();
        assert_eq!(drained, MAX_DRAIN_BYTES);
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
