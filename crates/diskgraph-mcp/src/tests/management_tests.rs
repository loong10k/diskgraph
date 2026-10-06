//! 保留原服务行为回归的全部断言；来源：原生 Rust MCP 内联测试迁移。
use super::support::{call, cargo_project, payload, service};
use crate::McpService;
use crate::protocol::ToolProfile;
use serde_json::json;

#[test]
fn an_index_job_is_reachable_after_the_call_returns() {
    let (mut service, _keep) = service(ToolProfile::Manage, "job");
    let (project, root) = cargo_project("job");
    let scope = register_scope(&service, &root);

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
    register_scope(&service, &root);
    let response = call(&mut service, "diskgraph_scope", json!({"action": "list"}));
    let data = payload(&response);
    assert!(!data["scopes"].as_array().unwrap().is_empty());
    drop(project);
}

/// 为管理入口建立真实持久 scope；本组只验证入队与列表，不制造已完成扫描或 revision。
/// 参数：service 为原本地授权服务，root 为真实目录；返回：已授权且持久化的范围标识。
fn register_scope(service: &McpService, root: &std::path::Path) -> String {
    let scope = service
        .engine()
        .register_scope(
            root,
            service.context.principal(),
            &service.authorizer().unwrap(),
        )
        .unwrap();
    assert!(service.engine().latest_revision(&scope).unwrap().is_none());
    scope.as_str().to_owned()
}
