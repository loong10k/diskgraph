//! HTTP debug UTF-8 边界与正文保密回归。

use crate::http::HttpRequest;
use crate::http_debug_request::format_request;
fn request(body: String) -> HttpRequest {
    HttpRequest {
        method: "POST".into(),
        path: "/mcp/中文".into(),
        query: String::new(),
        headers: Default::default(),
        body,
    }
}
#[test]
fn unicode_body_at_the_old_byte_boundary_never_panics() {
    let req = request(format!("{}中", "a".repeat(119)));
    let line = format_request(&req);
    assert!(line.contains("body_bytes=122"));
}
#[test]
fn debug_metadata_does_not_disclose_body_contents() {
    let req = request("sensitive-body-fixture".into());
    let line = format_request(&req);
    assert!(!line.contains(&req.body));
    assert!(line.contains("body_bytes=22"));
}
#[test]
fn empty_body_and_unicode_path_are_representable() {
    let line = format_request(&request(String::new()));
    assert!(line.contains("body_bytes=0"));
    assert!(line.contains("中文"));
}
