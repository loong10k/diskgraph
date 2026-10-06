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

