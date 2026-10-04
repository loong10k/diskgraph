//! D42 EC-02/EV-06 公共请求回归；来源：真实 MCP 认证、公开扫描与隔离 SQLite。
//! 只验证元数据采集任务的入口和持久身份，不将入队成功冒充原生占用验收。

use diskgraph_core::{Grant, Permission, PrincipalId, ScopeId};
use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits, HttpRequest};
use diskgraph_mcp::protocol::{Request, ToolProfile};
use diskgraph_mcp::{McpConfig, McpService, STDIO_PRINCIPAL};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 两个真实注册范围与普通文件的认证入口夹具。
/// 来源：DiskGraph 原生 Rust 集成测试；任务、归属和节点均通过公开 API 产生。
struct ProcessFixture {
    temp: tempfile::TempDir,
    service: McpService,
    scope: ScopeId,
    other_scope: ScopeId,
    revision: String,
    file_node: u64,
}

impl ProcessFixture {
    fn new() -> Self {
        // 真 Linux 正控使用实际 tmpfs epoch，不把外平台构造值当作扫描证据。
        #[cfg(target_os = "linux")]
        let temp = tempfile::tempdir_in("/dev/shm").expect("native tmpfs acceptance fixture");
        #[cfg(not(target_os = "linux"))]
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let other = temp.path().join("other");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&other).unwrap();
        std::fs::write(temp.path().join("empty-config"), b"").unwrap();
        let mut git = Command::new("git");
        git.env_clear();
        for key in ["PATH", "SystemRoot"] {
            if let Some(value) = std::env::var_os(key) {
                git.env(key, value);
            }
        }
        let initialized = git
            .args(["init", "-q"])
            .current_dir(&root)
            .env("HOME", temp.path())
            .env("USERPROFILE", temp.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", temp.path().join("empty-config"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(initialized.status.success(), "{initialized:?}");
        std::fs::write(root.join("held.txt"), b"ordinary fixture bytes\n").unwrap();
        let service = McpService::open(McpConfig {
            data_dir: temp.path().join("data"),
            profile: ToolProfile::Manage,
            ..McpConfig::default()
        })
        .unwrap();
        let principal = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
        let engine = service.engine();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let other_scope = engine
            .register_scope(&other, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine
            .run_job(&job.job_id, "process-entry-fixture")
            .unwrap();
        let revision = engine
            .revision_for_job(
                &job.job_id,
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let reader = engine.revision_reader().unwrap();
        let graph = reader
            .load(&reader.revision(&revision).unwrap().snapshot_id)
            .unwrap();
        let file_node = graph
            .nodes
            .iter()
            .find(|node| node.name == "held.txt")
            .unwrap()
            .id;
        drop(reader);
        Self {
            temp,
            service,
            scope,
            other_scope,
            revision,
            file_node,
        }
    }

    fn arguments(&self) -> Value {
        json!({"scope":self.scope.as_str(),"revision":self.revision,"node_id":self.file_node,"collector":"process"})
    }

    fn job_count(&self) -> i64 {
        rusqlite::Connection::open(self.temp.path().join("data/diskgraph-control.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap()
    }

    fn remote_call(
        &mut self,
        name: &str,
        arguments: Value,
        ceiling: &str,
        omitted_grant: Option<Permission>,
    ) -> (Value, PrincipalId, u64) {
        let auth = Authenticator::new(AuthConfig::single(
            "process-fixture",
            "diskgraph",
            b"isolated-process-test-key",
        ));
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300;
        let token = TokenMinter::new(b"isolated-process-test-key").mint(&TokenClaims {
            issuer: "process-fixture".into(),
            audience: "diskgraph".into(),
            subject: "collector".into(),
            expires_at_unix_seconds: expiry,
            scope: Some(ceiling.into()),
        });
        let identity = auth.authenticate(Some(&token)).unwrap();
        {
            let mut control = self.service.engine().control_store().unwrap();
            let version = control.policy_version().unwrap();
            for scope in [&self.scope, &self.other_scope] {
                for permission in [
                    Permission::MetadataRead,
                    Permission::OperationView,
                    Permission::IndexWrite,
                ] {
                    if omitted_grant.as_ref() == Some(&permission) {
                        continue;
                    }
                    control
                        .upsert_grant(&Grant {
                            principal: identity.principal.clone(),
                            permission,
                            scope: scope.clone(),
                            policy_version: version,
                        })
                        .unwrap();
                }
            }
        }
        let request = HttpRequest {
            method:"POST".into(), path:"/mcp".into(), query:String::new(),
            headers:HashMap::from([("authorization".into(),format!("Bearer {token}"))]),
            body:json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string(),
        };
        let response = http::handle_authenticated(
            &mut self.service,
            &request,
            &HttpLimits::default(),
            Some(&auth),
        );
        (
            serde_json::from_str(&response.body).unwrap(),
            identity.principal,
            expiry,
        )
    }
}

fn call(service: &mut McpService, args: Value) -> Value {
    service.handle(&Request {
        id: json!(1),
        method: "tools/call".into(),
        params: json!({"name":"diskgraph_sync","arguments":args}),
    })
}

fn payload(response: &Value) -> &Value {
    assert_eq!(response["result"]["isError"], false, "{response}");
    &response["result"]["structuredContent"]["data"]
}

const METADATA: &str = "metadata:read index:write operations:view";

#[test]
#[cfg(target_os = "linux")]
fn verified_metadata_only_process_sync_persists_original_authority_and_reopens() {
    let mut fixture = ProcessFixture::new();
    let count = fixture.job_count();
    let (response, principal, expiry) =
        fixture.remote_call("diskgraph_sync", fixture.arguments(), METADATA, None);
    let data = payload(&response);
    assert_eq!(data["state"], "queued");
    let job_id = data["job_id"].as_str().unwrap().to_owned();
    assert_eq!(fixture.job_count(), count + 1);
    fixture.service = McpService::open_remote(McpConfig {
        data_dir: fixture.temp.path().join("data"),
        profile: ToolProfile::Manage,
        ..McpConfig::default()
    })
    .unwrap();
    let job = fixture.service.engine().job_status(&job_id).unwrap();
    assert_eq!(job.principal, principal);
    assert_eq!(job.scope_id, fixture.scope);
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "process_evidence");
    assert_eq!(serde_json::to_value(job.state).unwrap(), "queued");
    let authority = fixture
        .service
        .engine()
        .control_store()
        .unwrap()
        .job_request_authority(&job_id)
        .unwrap()
        .unwrap();
    assert_eq!(authority.principal(), &principal);
    assert_eq!(authority.issuer(), Some("process-fixture"));
    assert_eq!(authority.transport(), "http");
    assert_eq!(authority.expires_at_unix_seconds(), Some(expiry));
    let ceiling = authority.capabilities().unwrap();
    assert!(ceiling.contains(&Permission::MetadataRead));
    assert!(ceiling.contains(&Permission::IndexWrite));
    assert!(!ceiling.contains(&Permission::ContentRead));
    assert_eq!(
        fixture
            .service
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(fixture.revision.as_str())
    );
    let (status, status_principal, _) =
        fixture.remote_call("diskgraph_status", json!({"job_id":job_id}), METADATA, None);
    assert_eq!(status_principal, principal);
    assert_eq!(payload(&status)["job_id"], job_id);
    assert_eq!(payload(&status)["scope_id"], fixture.scope.as_str());
    assert_eq!(payload(&status)["state"], "queued");
    assert_eq!(fixture.job_count(), count + 1);
}

#[test]
#[cfg(not(target_os = "linux"))]
fn unqualified_native_epoch_refuses_metadata_job_without_source_or_queue_changes() {
    let mut fixture = ProcessFixture::new();
    let count = fixture.job_count();
    let (response, _, _) =
        fixture.remote_call("diskgraph_sync", fixture.arguments(), METADATA, None);
    assert_eq!(
        response["error"]["data"]["business_code"], "unsupported",
        "{response}"
    );
    assert_eq!(fixture.job_count(), count);
    assert_eq!(
        fixture
            .service
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(fixture.revision.as_str())
    );
}

#[test]
fn process_sync_rejects_forged_fields_and_a_different_actual_revision_owner() {
    let mut fixture = ProcessFixture::new();
    let count = fixture.job_count();
    for (key, value) in [
        ("path", json!("/outside")),
        ("program", json!("arbitrary-probe")),
        ("argv", json!(["--read-memory"])),
        ("env", json!({"SECRET":"client"})),
        ("authority", json!({"principal":"admin"})),
        ("budget", json!({"bytes":u64::MAX})),
    ] {
        let mut args = fixture.arguments();
        args[key] = value;
        let (response, _, _) = fixture.remote_call("diskgraph_sync", args, METADATA, None);
        assert_eq!(response["error"]["code"], -32602, "{key}: {response}");
        assert_eq!(fixture.job_count(), count);
    }
    let mut args = fixture.arguments();
    args["scope"] = json!(fixture.other_scope.as_str());
    // 两范围均有真实授权；这里只拒绝与 revision 实际 owner 不一致的断言。
    let (response, _, _) = fixture.remote_call("diskgraph_sync", args, METADATA, None);
    assert_eq!(
        response["error"]["data"]["business_code"], "permission_denied",
        "{response}"
    );
    assert_eq!(fixture.job_count(), count);
    assert_eq!(
        fixture
            .service
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(fixture.revision.as_str())
    );
}

#[test]
fn process_sync_requires_metadata_and_index_in_both_original_token_and_live_grants() {
    for (ceiling, omitted_grant) in [
        ("metadata:read operations:view", None),
        ("index:write operations:view", None),
        (METADATA, Some(Permission::IndexWrite)),
        (METADATA, Some(Permission::MetadataRead)),
    ] {
        let mut fixture = ProcessFixture::new();
        let count = fixture.job_count();
        let (response, _, _) = fixture.remote_call(
            "diskgraph_sync",
            fixture.arguments(),
            ceiling,
            omitted_grant,
        );
        assert_eq!(
            response["error"]["data"]["business_code"], "permission_denied",
            "{response}"
        );
        assert_eq!(fixture.job_count(), count);
        assert_eq!(
            fixture
                .service
                .engine()
                .latest_revision(&fixture.scope)
                .unwrap()
                .as_deref(),
            Some(fixture.revision.as_str())
        );
    }
}

#[test]
fn ordinary_scan_and_git_keep_their_existing_durable_entry_contracts() {
    let mut fixture = ProcessFixture::new();
    let (response, principal, _) = fixture.remote_call(
        "diskgraph_sync",
        json!({"scope":fixture.scope.as_str()}),
        METADATA,
        None,
    );
    let scan_id = payload(&response)["job_id"].as_str().unwrap();
    let scan = fixture.service.engine().job_status(scan_id).unwrap();
    assert_eq!(scan.principal, principal);
    assert_eq!(serde_json::to_value(scan.kind).unwrap(), "sync");
    fixture
        .service
        .engine()
        .run_job_strict(scan_id, "process-scan-control")
        .unwrap();
    let revision = fixture
        .service
        .engine()
        .revision_for_job(
            scan_id,
            &principal,
            &fixture.service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_ne!(revision, fixture.revision);
    let local = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    fixture
        .service
        .engine()
        .set_content_read(&fixture.scope, &local, true)
        .unwrap();
    let reader = fixture.service.engine().revision_reader().unwrap();
    let graph = reader
        .load(&reader.revision(&revision).unwrap().snapshot_id)
        .unwrap();
    let root_node = graph.root().id;
    drop(reader);
    let response = call(
        &mut fixture.service,
        json!({"scope":fixture.scope.as_str(),"revision":revision,"node_id":root_node,"collector":"git"}),
    );
    let git_id = payload(&response)["job_id"].as_str().unwrap();
    let git = fixture.service.engine().job_status(git_id).unwrap();
    assert_eq!(serde_json::to_value(git.kind).unwrap(), "git_evidence");
    assert_eq!(serde_json::to_value(git.state).unwrap(), "queued");
    assert_eq!(
        fixture
            .service
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(revision.as_str())
    );
}
