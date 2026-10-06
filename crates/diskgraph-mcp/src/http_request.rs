use std::collections::HashMap;

/// HTTP 传输解析的请求对象，保留原始路径、查询串与有界正文。
/// 来源：原生 Rust MCP 传输路由契约；无 Java 对等对象。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    /// 未解析的原始查询串；legacy MCP 以查询参数标识消息会话。
    pub query: String,
    /// 小写字段名映射；重复单值头由解析器拒绝，XFF 合并保留有序代理链。
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl HttpRequest {
    /// 查询大小写不敏感的头字段。参数：name 为字段名；返回：原值借用或 None。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// 返回首个同名查询参数，不解码、不将客户端值解释为文件路径。
    /// 参数：name 为精确参数名；返回：原始值副本或 None，空值保持 Some。
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
