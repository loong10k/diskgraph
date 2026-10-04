//! 原服务测试的真实夹具与响应读取；来源：原生 MCP 内联测试迁移。
use crate::protocol::ToolProfile;
use crate::{McpConfig, McpService, STDIO_PRINCIPAL, protocol};
use diskgraph_core::PrincipalId;
use serde_json::{Value, json};

/// 建立独占数据目录中的真实 MCP 服务。参数：profile 为工具集合，label 为夹具名称。返回：服务及保活临时目录。
pub(crate) fn service(profile: ToolProfile, label: &str) -> (McpService, tempfile::TempDir) {
    let directory = tempfile::TempDir::with_prefix(format!("diskgraph-mcp-{label}-")).unwrap();
    let service = McpService::open(McpConfig {
        data_dir: directory.path().join("data"),
        profile,
        principal: PrincipalId::new(STDIO_PRINCIPAL).unwrap(),
        legacy_sse: false,
    })
    .unwrap();
    (service, directory)
}

/// 通过真实 handle 发送工具请求。参数：service、tool、arguments 为调用服务与业务输入。返回：原协议响应。
pub(crate) fn call(service: &mut McpService, tool: &str, arguments: Value) -> Value {
    let request = protocol::Request {
        id: json!(1),
        method: "tools/call".to_owned(),
        params: json!({"name": tool, "arguments": arguments}),
    };
    service.handle(&request)
}

/// 断言成功后借用 Envelope；协议断言作用于此层，业务结果在 data。参数：response 为响应。返回：成功 Envelope。
pub(crate) fn structured(response: &Value) -> &Value {
    assert!(
        response["error"].get("data").is_none(),
        "expected success, got: {response}"
    );
    &response["result"]["structuredContent"]
}

/// 借用成功 Envelope 的业务 data。参数：response 为协议响应。返回：业务数据。
pub(crate) fn payload(response: &Value) -> &Value {
    &structured(response)["data"]
}

/// 通过公开范围注册及扫描执行建立真实版本。参数：service 为服务，root 为原生根目录。返回：范围标识。
pub(crate) fn seed(service: &mut McpService, root: &std::path::Path) -> String {
    let scope_id = service
        .engine()
        .register_scope(
            root,
            service.context.principal(),
            &service.authorizer().unwrap(),
        )
        .unwrap();
    let job = service
        .engine()
        .index_scope(
            &scope_id,
            service.context.principal(),
            &service.authorizer().unwrap(),
        )
        .unwrap();
    service.engine().run_job(&job.job_id, "mcp-test").unwrap();
    scope_id.as_str().to_owned()
}

/// 创建带 target 产物的真实 Cargo 项目夹具。参数：label 为临时目录标签。返回：保活目录及项目路径。
pub(crate) fn cargo_project(label: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let workspace = tempfile::TempDir::with_prefix(format!("diskgraph-mcp-data-{label}-")).unwrap();
    let root = workspace.path().join("project");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(root.join("target").join("bin"), vec![0; 4096]).unwrap();
    (workspace, root)
}
