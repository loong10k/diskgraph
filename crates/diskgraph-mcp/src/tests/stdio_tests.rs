//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::service;
use crate::{protocol::ToolProfile, serve_stdio};

#[test]
fn stdio_serves_frames_and_logs_rejects_to_the_log_sink_only() {
    let (mut service, _keep) = service(ToolProfile::ReadMinimal, "stdio");
    let input = "not-json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n";
    let mut output = Vec::new();
    let mut log = Vec::new();
    serve_stdio(&mut service, input.as_bytes(), &mut output, &mut log).unwrap();
    let frames = String::from_utf8(output).unwrap();
    // Exactly one response frame: the malformed line produced no stdout.
    assert_eq!(frames.lines().count(), 1);
    assert!(frames.contains("diskgraph_explore"));
    // The rejection is recorded on the log sink, not on stdout.
    let logged = String::from_utf8(log).unwrap();
    assert!(logged.contains("frame_rejected"));
}

#[test]
fn batched_frames_are_refused_explicitly() {
    let (mut service, _keep) = service(ToolProfile::ReadMinimal, "batch");
    let mut output = Vec::new();
    let mut log = Vec::new();
    serve_stdio(
        &mut service,
        "[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}]\n".as_bytes(),
        &mut output,
        &mut log,
    )
    .unwrap();
    let frames = String::from_utf8(output).unwrap();
    assert!(frames.contains("-32600"));
    assert!(frames.contains("batched requests are not supported"));
}
