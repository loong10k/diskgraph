use serde_json::Value;

/// 连接循环按原字段写出的 HTTP 响应。
/// 来源：原生 Rust Streamable HTTP MCP-01 / MCP-02；无 Java 对等对象。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
    /// 向 MCP Streamable HTTP 客户端回传的会话标识。
    pub session: Option<String>,
}

impl HttpResponse {
    /// 参数：status 为状态码，value 为 JSON 值；返回：序列化正文及 JSON 内容类型的响应。
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string(),
            session: None,
        }
    }

    /// 接受 JSON-RPC 通知。参数：无；返回：空202响应，连接写出时不附正文或 Content-Type。
    pub fn accepted() -> Self {
        Self {
            status: 202,
            content_type: "application/json",
            body: String::new(),
            session: None,
        }
    }

    /// 参数：无；返回：空202 event-stream响应，供每次POST都按流读取的客户端使用。
    pub fn accepted_stream() -> Self {
        Self {
            status: 202,
            content_type: "text/event-stream",
            body: String::new(),
            session: None,
        }
    }

    /// 参数：status 为状态码，body 为文本正文；返回：UTF-8纯文本响应，不分配会话。
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
            session: None,
        }
    }
}
