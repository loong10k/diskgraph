//! 请求分发、路由和内部处理器的原始期限回归，使用真实控制锁。
use crate::{McpConfig, McpService};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn explicit_revision_dispatch_refuses_before_control_owner_release() {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let owner = service.engine().control_store().unwrap();
    let mut request = service.clone();
    let (finished, result) = mpsc::channel();
    let deadline = std::time::Instant::now() + Duration::from_millis(50);
    let worker = std::thread::spawn(move || {
        let outcome = request.dispatch("C13", &serde_json::json!({"revision":"missing"}), deadline);
        finished
            .send(outcome.err().map(|error| crate::business_of(&error.error)))
            .unwrap();
    });
    // 锁在观察窗口内保持占用；即使失败，也先释放再回收工作线程。
    let bounded = result.recv_timeout(Duration::from_millis(300));
    drop(owner);
    worker.join().unwrap();
    assert!(
        matches!(
            bounded,
            Ok(Some(diskgraph_core::BusinessError::BudgetExceeded))
        ),
        "dispatch waited for lock release: {bounded:?}"
    );
}

#[test]
fn transport_deadline_is_not_renewed_before_tool_validation() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let request = crate::protocol::Request {
        id: serde_json::json!(912),
        method: "tools/call".into(),
        params: serde_json::json!({}),
    };
    let response = service.handle_until(
        &request,
        std::time::Instant::now() - Duration::from_millis(1),
    );
    assert_eq!(response["id"], request.id);
    assert_eq!(
        response["error"]["data"]["business_code"],
        "budget_exceeded"
    );
}

#[test]
fn longer_transport_window_cannot_enlarge_tool_budget() {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let owner = service.engine().control_store().unwrap();
    let mut request_service = service.clone();
    let request = crate::protocol::Request {
        id: serde_json::json!(913),
        method: "tools/call".into(),
        params: serde_json::json!({"name":"diskgraph_related", "arguments":{"revision":"missing", "entity":"missing"}}),
    };
    let (finished, result) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        finished
            .send(request_service.handle_until(
                &request,
                std::time::Instant::now() + Duration::from_secs(10),
            ))
            .unwrap();
    });
    let bounded = result.recv_timeout(Duration::from_millis(
        diskgraph_core::QueryBudget::default().deadline_ms + 500,
    ));
    drop(owner);
    worker.join().unwrap();
    let response = bounded.expect("transport enlarged tool budget while owner remained locked");
    assert_eq!(
        response["error"]["data"]["business_code"], "budget_exceeded",
        "{response}"
    );
}

fn scope_authorization_stops_before_owner_release(mode: u8) {
    let dir = tempfile::tempdir().unwrap();
    let service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let owner = service.engine().control_store().unwrap();
    let request = service.clone();
    let (finished, result) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        let outcome = match mode {
            0 => request
                .resolve_scope_until(&serde_json::json!({"scope":"missing"}), deadline)
                .map(|_| ()),
            1 => request
                .resolve_scope_until(&serde_json::json!({}), deadline)
                .map(|_| ()),
            3 => request
                .require_revision_until(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing"}),
                    deadline,
                )
                .map(|_| ()),
            4 => request
                .related_tool(
                    &serde_json::json!({"revision":"missing","entity":"missing"}),
                    deadline,
                )
                .map(|_| ()),
            5 => request
                .explain_tool(
                    &serde_json::json!({"revision":"missing","entity":"missing"}),
                    deadline,
                )
                .map(|_| ()),
            6 => request
                .impact_tool(
                    &serde_json::json!({"revision":"missing","entity":"missing"}),
                    deadline,
                )
                .map(|_| ()),
            7 => request
                .candidates_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing"}),
                    deadline,
                )
                .map(|_| ()),
            8 => request
                .history_tool(
                    "C06",
                    &serde_json::json!({"before":"a","after":"b"}),
                    deadline,
                )
                .map(|_| ()),
            9 => request
                .history_tool(
                    "C07",
                    &serde_json::json!({"before":"a","after":"b"}),
                    deadline,
                )
                .map(|_| ()),
            10 => request
                .explore_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing","pattern":"x"}),
                    deadline,
                )
                .map(|_| ()),
            11 => request
                .search_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing","pattern":"x"}),
                    deadline,
                )
                .map(|_| ()),
            12 => request
                .node_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing","pattern":"x"}),
                    deadline,
                )
                .map(|_| ()),
            13 => request
                .children_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing","pattern":"x"}),
                    deadline,
                )
                .map(|_| ()),
            14 => request
                .top_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({"revision":"missing","pattern":"x"}),
                    deadline,
                )
                .map(|_| ()),
            15 => request
                .scope_tool(&serde_json::json!({}), deadline)
                .map(|_| ()),
            16 => request
                .status_tool(&serde_json::json!({}), deadline)
                .map(|_| ()),
            17 => request
                .snapshots_tool(
                    &Some(diskgraph_core::ScopeId::new("missing").unwrap()),
                    &serde_json::json!({}),
                    deadline,
                )
                .map(|_| ()),
            _ => request.require_until(
                &diskgraph_core::Permission::MetadataRead,
                &diskgraph_engine::admin_scope(),
                deadline,
            ),
        };
        finished
            .send(outcome.err().map(|error| crate::business_of(&error)))
            .unwrap();
    });
    let bounded = result.recv_timeout(Duration::from_millis(300));
    drop(owner);
    worker.join().unwrap();
    assert!(
        matches!(
            bounded,
            Ok(Some(diskgraph_core::BusinessError::BudgetExceeded))
        ),
        "mode {mode}: {bounded:?}"
    );
}

#[test]
fn explicit_scope_lookup_respects_original_deadline() {
    scope_authorization_stops_before_owner_release(0);
}
#[test]
fn default_scope_lookup_respects_original_deadline() {
    scope_authorization_stops_before_owner_release(1);
}
#[test]
fn common_permission_lookup_respects_original_deadline() {
    scope_authorization_stops_before_owner_release(2);
}

#[test]
fn revision_lookup_respects_original_deadline() {
    scope_authorization_stops_before_owner_release(3);
}

#[test]
fn modern_post_does_not_refresh_exhausted_routing_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut service = McpService::open(McpConfig {
        data_dir: dir.path().join("data"),
        ..McpConfig::default()
    })
    .unwrap();
    let request = crate::http::HttpRequest {
        method: "POST".into(),
        path: "/mcp".into(),
        query: String::new(),
        headers: std::collections::HashMap::new(),
        body: serde_json::json!({"jsonrpc":"2.0","id":914,"method":"tools/call","params":{}})
            .to_string(),
    };
    let response = crate::http::handle_secured(
        &mut service,
        &request,
        &crate::http::HttpLimits {
            read_timeout: Duration::ZERO,
            ..crate::http::HttpLimits::default()
        },
        &crate::http::Security::local(None),
    );
    let body: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(body["id"], 914);
    assert_eq!(
        body["error"]["data"]["business_code"], "budget_exceeded",
        "{body}"
    );
}

#[test]
fn related_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(4);
}
#[test]
fn explain_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(5);
}
#[test]
fn impact_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(6);
}
#[test]
fn candidates_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(7);
}

#[test]
fn history_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(8);
}
#[test]
fn growth_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(9);
}

#[test]
fn explore_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(10);
}

#[test]
fn search_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(11);
}

#[test]
fn node_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(12);
}

#[test]
fn children_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(13);
}

#[test]
fn top_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(14);
}

#[test]
fn scope_management_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(15);
}
#[test]
fn status_management_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(16);
}
#[test]
fn snapshots_management_handler_cannot_wait_past_request_deadline() {
    scope_authorization_stops_before_owner_release(17);
}
