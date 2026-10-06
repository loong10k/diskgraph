//! MCP 显式受信宿主启动与外部 Recovery 生命周期合同；来源：PF-06 原生 Rust。
//! held 普通文件只证明配置材料和两库启动，不执行它，不证明镜像、scanner 或 OS 退出资格。

use diskgraph_core::{Authorizer, Decision, Permission, PrincipalId};
use diskgraph_engine::{
    ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget, admin_scope,
};
use diskgraph_mcp::http::{HttpLimits, HttpRequest};
use diskgraph_mcp::protocol::ToolProfile;
use diskgraph_mcp::{McpConfig, McpService, http};
use diskgraph_scan_worker::ProtocolLimits;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

fn host(directory: &Path) -> ScanWorkerHost {
    let image_path = directory.join("held-startup-material");
    let bytes = b"ordinary held material; no execution qualification";
    std::fs::write(&image_path, bytes).unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(
        Sha256::digest(bytes).into(),
        u64::try_from(bytes.len()).unwrap(),
    )
    .unwrap();
    let limits = ProtocolLimits {
        max_frame_bytes: 64 << 10,
        max_stream_bytes: 1 << 20,
        max_nodes: 100,
        max_depth: 8,
    };
    let budget = ScanWorkerRuntimeBudget::new(limits, 64 << 10, 2).unwrap();
    ScanWorkerHost::new(File::open(image_path).unwrap(), expected, budget).unwrap()
}

fn config(directory: &Path) -> McpConfig {
    McpConfig {
        data_dir: directory.join("data"),
        profile: ToolProfile::Manage,
        principal: PrincipalId::new("actual-mcp-host-principal").unwrap(),
        legacy_sse: true,
    }
}

fn database(data_dir: &Path) -> Connection {
    // 使用实际服务创建的同一个控制库，只读查询已提交行，不镜像启动算法。
    Connection::open_with_flags(
        data_dir.join("diskgraph-control.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
}

fn durable_policy(data_dir: &Path) -> Option<(i64, i64)> {
    database(data_dir)
        .query_row(
            "SELECT version, revoked FROM policy WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap()
}

fn durable_grants(data_dir: &Path) -> Vec<(String, String, String, i64)> {
    let connection = database(data_dir);
    let mut statement = connection
        .prepare(
            "SELECT principal_id, permission, scope_id, policy_version FROM grants
             ORDER BY principal_id, permission, scope_id, policy_version",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn trusted_stdio_host_bootstraps_once_and_service_clones_share_external_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let configuration = config(directory.path());
    let (service, recovery) =
        McpService::open_with_scan_worker(configuration.clone(), host(directory.path())).unwrap();
    assert_eq!(service.profile(), ToolProfile::Manage);
    assert_eq!(service.engine().data_dir(), configuration.data_dir);
    assert_eq!(
        service.engine().policy_authorizer().unwrap().decide(
            &configuration.principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        ),
        Decision::Allowed
    );
    assert_eq!(durable_policy(&configuration.data_dir), Some((1, 0)));
    let grants = durable_grants(&configuration.data_dir);
    assert_eq!(grants.len(), 4);
    assert!(grants.iter().all(|row| {
        row.0 == configuration.principal.as_str() && row.2 == "diskgraph-admin" && row.3 == 1
    }));
    assert_eq!(
        grants.iter().map(|row| row.1.as_str()).collect::<Vec<_>>(),
        [
            "index:write",
            "metadata:read",
            "operations:view",
            "scope:admin",
        ]
    );
    let server = service.engine().server_id().unwrap();
    let cloned = service.clone();
    assert!(std::ptr::eq(service.engine(), cloned.engine()));
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    drop(service);
    assert_eq!(cloned.engine().server_id().unwrap(), server);
    drop(cloned);
    // Recovery 保存在服务及其 clone 生命周期之外；本案没有 child 出生或 wait 证明。
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(recovery.drain().unwrap());
    let reopened = McpService::open(configuration.clone()).unwrap();
    assert_eq!(reopened.engine().server_id().unwrap(), server);
    assert_eq!(durable_grants(&configuration.data_dir), grants);
    drop(reopened);
    assert!(recovery.drain().unwrap());
}

#[test]
fn remote_host_startup_does_not_publish_policy_or_bootstrap_administration() {
    let directory = tempfile::tempdir().unwrap();
    let configuration = config(directory.path());
    let (service, recovery) =
        McpService::open_remote_with_scan_worker(configuration.clone(), host(directory.path()))
            .unwrap();
    assert_eq!(durable_policy(&configuration.data_dir), None);
    assert!(durable_grants(&configuration.data_dir).is_empty());
    assert!(matches!(
        service.engine().policy_authorizer().unwrap().decide(
            &configuration.principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        ),
        Decision::Denied(_)
    ));
    let server = service.engine().server_id().unwrap();
    drop(service);
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(recovery.drain().unwrap());
    let reopened = McpService::open_remote(configuration.clone()).unwrap();
    assert_eq!(reopened.engine().server_id().unwrap(), server);
    assert_eq!(durable_policy(&configuration.data_dir), None);
    assert!(durable_grants(&configuration.data_dir).is_empty());
}

#[test]
fn remote_host_keeps_existing_grants_but_does_not_inherit_local_request_privileges() {
    let directory = tempfile::tempdir().unwrap();
    let configuration = config(directory.path());
    let local = McpService::open(configuration.clone()).unwrap();
    let server = local.engine().server_id().unwrap();
    drop(local);
    let before_policy = durable_policy(&configuration.data_dir);
    let before_grants = durable_grants(&configuration.data_dir);
    assert_eq!(before_policy, Some((1, 0)));
    assert_eq!(before_grants.len(), 4);
    let (mut remote, recovery) =
        McpService::open_remote_with_scan_worker(configuration.clone(), host(directory.path()))
            .unwrap();
    assert_eq!(remote.engine().server_id().unwrap(), server);
    assert_eq!(durable_policy(&configuration.data_dir), before_policy);
    assert_eq!(durable_grants(&configuration.data_dir), before_grants);
    // 同主体已有真实本地 grant，不代表未认证远程请求能继承它。
    assert_eq!(
        remote.engine().policy_authorizer().unwrap().decide(
            &configuration.principal,
            &Permission::ScopeAdmin,
            &admin_scope(),
        ),
        Decision::Allowed
    );
    let request = HttpRequest {
        method: "POST".into(),
        path: "/mcp".into(),
        query: String::new(),
        headers: HashMap::new(),
        body: r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"diskgraph_scope","arguments":{"action":"list"}}}"#.into(),
    };
    assert_eq!(
        http::handle_authenticated(&mut remote, &request, &HttpLimits::default(), None).status,
        401
    );
    drop(remote);
    assert_eq!(recovery.occupied_slots().unwrap(), 0);
    assert!(recovery.drain().unwrap());
    assert_eq!(durable_policy(&configuration.data_dir), before_policy);
    assert_eq!(durable_grants(&configuration.data_dir), before_grants);
}
