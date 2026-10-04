//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::{call, cargo_project, payload, seed, service};
use crate::protocol::ToolProfile;
use serde_json::json;

#[test]
fn an_index_job_is_reachable_after_the_call_returns() {
    let (mut service, _keep) = service(ToolProfile::Manage, "job");
    let (project, root) = cargo_project("job");
    let scope = seed(&mut service, &root);

    let response = call(&mut service, "diskgraph_index", json!({"scope": scope}));
    let data = payload(&response);
    let job_id = data["job_id"].as_str().expect("a durable job id");
    assert_eq!(data["state"], "queued");
    assert_eq!(data["poll_with"], "diskgraph_status");

    // A separate call (simulating a reconnect) reports the durable record.
    // The job is created but not run, so the state is exactly what the
    // control store holds.
    let status = call(&mut service, "diskgraph_status", json!({"job_id": job_id}));
    assert_eq!(payload(&status)["state"], "queued");
    assert_eq!(payload(&status)["scope_id"], scope.as_str());
    drop(project);
}

#[test]
fn the_manage_profile_serves_scope_listing() {
    let (mut service, _keep) = service(ToolProfile::Manage, "manage-scope");
    let (project, root) = cargo_project("manage-scope");
    seed(&mut service, &root);
    let response = call(&mut service, "diskgraph_scope", json!({"action": "list"}));
    let data = payload(&response);
    assert!(!data["scopes"].as_array().unwrap().is_empty());
    drop(project);
}
