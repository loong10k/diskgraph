use std::time::Duration;

/// HTTP 传输在执行请求前应用的资源上限。
/// 来源：原生 Rust MCP-06 / RT-02；无 Java 对等对象。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpLimits {
    /// 接受的单次请求正文最大字节数。
    pub max_body_bytes: usize,
    /// 单次请求生成的响应最大字节数。
    pub max_response_bytes: usize,
    /// 每条连接关闭前最多处理的请求数。
    pub max_requests_per_connection: usize,
    /// 最大并发连接数；超限返回503，不建立无界等待队列。
    pub max_concurrent_connections: usize,
    /// 同一客户端所有连接共享的每秒请求令牌桶速率。
    pub max_requests_per_second_per_client: u32,
    /// 完整请求行、头和正文共享的绝对读取期限；收到新字节不续期。
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
