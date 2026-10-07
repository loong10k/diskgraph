//! HTTP debug请求元信息格式化。
use crate::http::HttpRequest;
/// 参数：已解析请求；返回：有限debug元信息，生产调用不应因UTF-8边界panic。
pub(crate) fn format_request(request: &HttpRequest) -> String {
    format!(
        "[http-debug] {:?} {:?} body_bytes={}",
        request.method,
        request.path,
        request.body.len()
    )
}
