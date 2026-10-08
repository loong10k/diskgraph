//! 对真实 MCP 查询 envelope 及首次授权之前的整次期限作回归。

use crate::protocol::ToolProfile;
use crate::tests::{call, cargo_project, seed, service};
use diskgraph_core::{QueryBudget, ScopeId};
use serde_json::json;
use std::cell::RefCell;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

thread_local! {
    static STARTED: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
    static BEFORE_REPLY: RefCell<Option<ReplyHook>> = const { RefCell::new(None) };
}
type ReplyHook = Box<dyn FnOnce(&crate::McpService)>;

/// 安装或清除本测试线程的实际编码观察点；参数为一次性回调，不进入生产状态。
pub(super) fn install_before_reply(hook: Option<ReplyHook>) {
    BEFORE_REPLY.with(|slot| *slot.borrow_mut() = hook);
}

pub(super) fn before_reply(service: &crate::McpService) {
    if let Some(hook) = BEFORE_REPLY.with(|slot| slot.borrow_mut().take()) {
        hook(service);
    }
}

pub(super) fn request_started() {
    if let Some(sender) = STARTED.with(|slot| slot.borrow_mut().take()) {
        sender.send(()).unwrap();
    }
}

#[test]
fn initial_mcp_authorization_wait_cannot_reset_the_candidate_deadline() {
    let (mut service, _data) = service(ToolProfile::ReadFull, "candidate-request-deadline");
    let (_project, root) = cargo_project("candidate-request-deadline");
    let scope = seed(&mut service, &root);
    let engine = service.engine.clone();
    let guard = engine.control_store().unwrap();
    let (sender, receiver) = channel();
    let worker = std::thread::spawn(move || {
        STARTED.with(|slot| *slot.borrow_mut() = Some(sender));
        call(
            &mut service,
            "diskgraph_candidates",
            json!({"scope": scope, "target_bytes": 0}),
        )
    });
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    // 真实控制连接保持占用；首个授权和请求调度耗时必须属于同一次期限。
    std::thread::sleep(Duration::from_millis(
        QueryBudget::default().deadline_ms + 100,
    ));
    drop(guard);
    let response = worker.join().unwrap();
    let envelope = &response["result"]["structuredContent"];
    assert!(
        envelope["data"]["complete"] != true,
        "MCP returned a late complete candidate response: {response}"
    );
    if response["result"]["isError"] == false {
        assert_eq!(envelope["data"]["truncated"], "deadline");
    }
}

#[test]
fn escaped_impact_fits_the_real_mcp_structured_envelope() {
    let (mut service, data) = service(ToolProfile::ReadFull, "impact-encoded-envelope");
    let (_project, root) = cargo_project("impact-encoded-envelope");
    let scope = seed(&mut service, &root);
    let revision = service
        .engine
        .latest_revision(&ScopeId::new(scope.clone()).unwrap())
        .unwrap()
        .unwrap();
    let connection = rusqlite::Connection::open(data.path().join("data/diskgraph.sqlite")).unwrap();
    let source: String = connection
        .query_row(
            "SELECT source_entity_id FROM relations WHERE relation = 'rebuildable_by' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(connection.execute(
        "UPDATE relations SET target_entity_id = ?1, edge_json = json_set(edge_json, '$.target_entity_id', ?1) WHERE source_entity_id = ?2 AND relation = 'rebuildable_by'",
        [&"\0".repeat(11_000), &source],
    ).unwrap() > 0);
    let response = call(
        &mut service,
        "diskgraph_impact",
        json!({"scope":scope,"revision":revision,"entity":source}),
    );
    let envelope = &response["result"]["structuredContent"];
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert!(
        envelope.to_string().len() <= QueryBudget::default().max_response_bytes,
        "oversized structured envelope"
    );
    assert_eq!(envelope["data"]["complete"], false, "{response}");
    assert_eq!(envelope["data"]["truncated"], "byte_limit");
}

#[test]
fn final_mcp_envelope_cannot_escape_after_scope_revocation() {
    let (mut service, _data) = service(ToolProfile::ReadFull, "mcp-terminal-scope");
    let (_project, root) = cargo_project("mcp-terminal-scope");
    let scope = seed(&mut service, &root);
    let revoked = ScopeId::new(scope.clone()).unwrap();
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |service| {
            service
                .engine
                .revoke_scope(
                    &revoked,
                    service.context.principal(),
                    &service.authorizer().unwrap(),
                )
                .unwrap();
        }))
    });
    let reply = call(
        &mut service,
        "diskgraph_candidates",
        json!({"scope":scope,"target_bytes":0}),
    );
    assert_eq!(reply["error"]["code"], -32001, "{reply}");
    assert_eq!(reply["error"]["data"]["business_code"], "permission_denied");
    assert_eq!(reply["error"]["data"]["exit_code"], 3);
    assert!(reply.get("result").is_none());
}
