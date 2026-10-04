//! 真实 MCP 状态入口的初次授权锁竞争；来源：原始工具请求与唯一 Engine 控制连接。
use crate::protocol::ToolProfile;
use crate::tests::{call, service};
use serde_json::json;
use std::time::Duration;

#[test]
fn mcp_status_authorizer_does_not_block_beyond_original_request_deadline() {
    let (mut service, _keep) = service(ToolProfile::Manage, "status-authorizer-deadline");
    let engine = service.engine.clone();
    let guard = engine.control_store().unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        let response = call(
            &mut service,
            "diskgraph_status",
            json!({"job_id":"not-yet-read"}),
        );
        finished.send(response).unwrap();
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    // 原请求默认一秒，持锁方继续存活；不能释放锁后才给迟到预算错误。
    let observed = result.recv_timeout(Duration::from_secs(2));
    drop(guard);
    worker.join().unwrap();
    let response =
        observed.expect("public MCP authorization must time out while owner still holds control");
    assert_eq!(
        response["error"]["data"]["business_code"], "budget_exceeded",
        "{response}"
    );
}
