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

