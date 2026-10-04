//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::{call, cargo_project, seed, service};
use crate::protocol::{self, ToolProfile};
use serde_json::json;

#[test]
fn initialize_and_tool_listing_work_over_the_protocol() {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "list");
    let initialize = service.handle(&protocol::Request {
        id: json!(1),
        method: "initialize".to_owned(),
        params: json!({"protocolVersion": protocol::PROTOCOL_VERSION}),
    });
    assert_eq!(initialize["result"]["serverInfo"]["name"], "diskgraph");
    assert_eq!(
        initialize["result"]["protocolVersion"],
        protocol::PROTOCOL_VERSION
    );

    let listed = service.handle(&protocol::Request {
        id: json!(2),
        method: "tools/list".to_owned(),
        params: json!({}),
    });
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(!tools.is_empty());
    assert!(tools.iter().all(|tool| tool["name"].is_string()));
}

#[test]
fn tool_calls_reject_unknown_and_mistyped_arguments() {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "strict-arguments");
    let (_project, root) = cargo_project("strict-arguments");
    let scope = seed(&mut service, &root);
    for args in [
        json!({"scope":scope,"node_id":"2"}),
        json!({"scope":scope,"node_id":-1}),
        json!({"scope":scope,"node_id":0}),
        json!({"scope":scope,"typo":2}),
        json!({"scope":scope,"revision":null}),
        json!([]),
    ] {
        let response = call(&mut service, "diskgraph_node", args);
        assert_eq!(response["error"]["code"], -32602, "{response}");
    }
}

#[test]
fn unknown_methods_and_tools_answer_protocol_errors() {
    let (mut service, _keep) = service(ToolProfile::All, "errors");
    let response = service.handle(&protocol::Request {
        id: json!(9),
        method: "nope".to_owned(),
        params: json!({}),
    });
    assert_eq!(response["error"]["code"], -32601);
    let response = call(&mut service, "diskgraph_nope", json!({}));
    assert_eq!(response["error"]["code"], -32601);
}

#[test]
fn a_tool_outside_the_active_profile_is_refused_not_merely_hidden() {
    let (mut service, _keep) = service(ToolProfile::ReadFull, "profile-gate");
    // read-full does not advertise scope management, so calling it must
    // fail even though the tool exists in the catalog.
    let listed = service.handle(&protocol::Request {
        id: json!(1),
        method: "tools/list".to_owned(),
        params: json!({}),
    });
    let names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    assert!(!names.contains(&"diskgraph_scope".to_owned()));

    let response = call(&mut service, "diskgraph_scope", json!({"action": "list"}));
    assert_eq!(response["error"]["data"]["business_code"], "unsupported");
}
