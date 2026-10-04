//! D37 C03/EC-02/04 公共 MCP 入口回归；来源：真实 Rust 服务、原生 Git 与隔离 SQLite。
//! 本文件覆盖参数、授权交集、持久入队、真实执行/发布及重连状态；不替代三平台原生验收。

use diskgraph_core::{Grant, Permission, PrincipalId, ScopeId};
use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits, HttpRequest};
use diskgraph_mcp::protocol::{Request, ToolProfile};
use diskgraph_mcp::{McpConfig, McpService, STDIO_PRINCIPAL};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 原生 Git + 实际扫描产生的 revision/node；不伪造 scope ownership 或定位。
/// 来源：DiskGraph 原生 Rust 集成测试，无 Java 对应实现。
struct GitFixture {
    temp: tempfile::TempDir,
    service: McpService,
    scope: ScopeId,
    revision: String,
    node_id: u64,
}

impl GitFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(temp.path().join("empty-config"), "").unwrap();
        git(&root, temp.path(), &["init", "-q"]);
        std::fs::write(root.join("tracked.txt"), "original\n").unwrap();
        git(&root, temp.path(), &["add", "tracked.txt"]);
        git(
            &root,
            temp.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        std::fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
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
        let job = engine
            .index_scope(&scope, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine.run_job(&job.job_id, "git-entry-fixture").unwrap();
        let revision = engine
            .revision_for_job(
                &job.job_id,
                &principal,
                &engine.policy_authorizer().unwrap(),
            )
            .unwrap();
        let reader = engine.revision_reader().unwrap();
        let snapshot = reader.revision(&revision).unwrap().snapshot_id;
        let node_id = reader.load(&snapshot).unwrap().root().id;
        drop(reader);
        Self {
            temp,
            service,
            scope,
            revision,
            node_id,
        }
    }

    fn arguments(&self) -> Value {
        json!({"scope":self.scope.as_str(), "revision":self.revision, "node_id":self.node_id, "collector":"git"})
    }

    fn job_count(&self) -> i64 {
        rusqlite::Connection::open(self.temp.path().join("data/diskgraph-control.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap()
    }

    fn remote_call(
        &mut self,
        arguments: Value,
        ceiling: &str,
        db_content: bool,
    ) -> (Value, PrincipalId) {
        let auth = Authenticator::new(AuthConfig::single(
            "git-fixture",
            "diskgraph",
            b"isolated-test-key",
        ));
        let token = TokenMinter::new(b"isolated-test-key").mint(&TokenClaims {
            issuer: "git-fixture".into(),
            audience: "diskgraph".into(),
            subject: "collector".into(),
            expires_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 300,
            scope: Some(ceiling.into()),
        });
        let identity = auth.authenticate(Some(&token)).unwrap();
        {
            let mut control = self.service.engine().control_store().unwrap();
            let version = control.policy_version().unwrap();
            for permission in [
                Permission::MetadataRead,
                Permission::IndexWrite,
                Permission::OperationView,
            ] {
                control
                    .upsert_grant(&Grant {
                        principal: identity.principal.clone(),
                        permission,
                        scope: self.scope.clone(),
                        policy_version: version,
                    })
                    .unwrap();
            }
        }
        self.service
            .engine()
            .set_content_read(&self.scope, &identity.principal, db_content)
            .unwrap();
        let request = HttpRequest {
            method: "POST".into(), path: "/mcp".into(), query: String::new(),
            headers: HashMap::from([("authorization".into(), format!("Bearer {token}"))]),
            body: json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"diskgraph_sync", "arguments":arguments}}).to_string(),
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
        )
    }
}

fn git(root: &Path, home: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    command.env_clear();
    for key in ["PATH", "SystemRoot"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = command
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty-config"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn call(service: &mut McpService, arguments: Value) -> Value {
    service.handle(&Request {
        id: json!(1),
        method: "tools/call".into(),
        params: json!({"name":"diskgraph_sync", "arguments":arguments}),
    })
}

fn payload(response: &Value) -> &Value {
    assert_eq!(response["result"]["isError"], false, "{response}");
    &response["result"]["structuredContent"]["data"]
}

const ALL: &str = "metadata:read index:write content:read operations:view";

#[test]
fn sync_schema_discovers_explicit_git_target_without_client_execution_controls() {
    let mut fixture = GitFixture::new();
    let response = fixture.service.handle(&Request {
        id: json!(1),
        method: "tools/list".into(),
        params: json!({}),
    });
    let schema = &response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "diskgraph_sync")
        .unwrap()["inputSchema"];
    assert_eq!(schema["properties"]["collector"]["enum"], json!(["git"]));
    assert_eq!(schema["properties"]["revision"]["type"], "string");
    assert_eq!(schema["properties"]["node_id"]["type"], "integer");
    assert_eq!(schema["additionalProperties"], false);
    for key in ["path", "argv", "authority", "network"] {
        assert!(schema["properties"].get(key).is_none());
    }
}

#[test]
fn authenticated_git_sync_returns_a_durable_job_for_the_verified_principal() {
    let mut fixture = GitFixture::new();
    let count = fixture.job_count();
    let (response, principal) = fixture.remote_call(fixture.arguments(), ALL, true);
    let data = payload(&response);
    assert_eq!(data["state"], "queued");
    let job_id = data["job_id"].as_str().unwrap();
    assert_eq!(fixture.job_count(), count + 1);
    let reopened = McpService::open_remote(McpConfig {
        data_dir: fixture.temp.path().join("data"),
        profile: ToolProfile::Manage,
        ..McpConfig::default()
    })
    .unwrap();
    let job = reopened.engine().job_status(job_id).unwrap();
    assert_eq!(job.scope_id, fixture.scope);
    assert_eq!(job.principal, principal);
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "git_evidence");
    assert_eq!(serde_json::to_value(job.state).unwrap(), "queued");
    assert_eq!(
        reopened
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(fixture.revision.as_str()),
        "enqueue must not publish a replacement revision"
    );
}

#[test]
fn git_sync_requires_content_read_in_the_verified_token_ceiling() {
    let mut fixture = GitFixture::new();
    let count = fixture.job_count();
    let (response, _) = fixture.remote_call(
        fixture.arguments(),
        "metadata:read index:write operations:view",
        true,
    );
    assert_eq!(
        response["error"]["data"]["business_code"], "permission_denied",
        "{response}"
    );
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn git_sync_requires_content_read_in_live_database_grants() {
    let mut fixture = GitFixture::new();
    let count = fixture.job_count();
    let (response, _) = fixture.remote_call(fixture.arguments(), ALL, false);
    assert_eq!(
        response["error"]["data"]["business_code"], "permission_denied",
        "{response}"
    );
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn git_sync_rejects_a_registered_scope_that_does_not_own_the_revision() {
    let mut fixture = GitFixture::new();
    let other_root = fixture.temp.path().join("other");
    std::fs::create_dir(&other_root).unwrap();
    let principal = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    let engine = fixture.service.engine();
    let other = engine
        .register_scope(
            &other_root,
            &principal,
            &engine.policy_authorizer().unwrap(),
        )
        .unwrap();
    engine.set_content_read(&other, &principal, true).unwrap();
    engine
        .set_content_read(&fixture.scope, &principal, true)
        .unwrap();
    let count = fixture.job_count();
    let mut args = fixture.arguments();
    args["scope"] = json!(other.as_str());
    let response = call(&mut fixture.service, args);
    assert_eq!(
        response["error"]["data"]["business_code"], "permission_denied",
        "{response}"
    );
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn git_sync_cannot_accept_client_paths_commands_or_authority() {
    let mut fixture = GitFixture::new();
    let count = fixture.job_count();
    for (key, value) in [
        ("path", json!("/outside")),
        ("argv", json!(["status"])),
        ("authority", json!({"principal":"admin"})),
        ("network", json!(true)),
    ] {
        let mut args = fixture.arguments();
        args[key] = value;
        let response = call(&mut fixture.service, args);
        assert_eq!(response["error"]["code"], -32602, "{key}: {response}");
    }
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn sync_without_a_collector_keeps_the_existing_scan_contract() {
    let mut fixture = GitFixture::new();
    let response = call(
        &mut fixture.service,
        json!({"scope":fixture.scope.as_str()}),
    );
    let job_id = payload(&response)["job_id"].as_str().unwrap();
    let job = fixture.service.engine().job_status(job_id).unwrap();
    assert_eq!(serde_json::to_value(job.kind).unwrap(), "sync");
    fixture
        .service
        .engine()
        .run_job(job_id, "legacy-sync-control")
        .unwrap();
    let principal = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    let revision = fixture
        .service
        .engine()
        .revision_for_job(
            job_id,
            &principal,
            &fixture.service.engine().policy_authorizer().unwrap(),
        )
        .unwrap();
    assert_ne!(revision, fixture.revision);
}

#[test]
fn authenticated_git_job_execution_is_reconnectable_with_the_actual_receipt() {
    let mut fixture = GitFixture::new();
    let (response, principal) = fixture.remote_call(fixture.arguments(), ALL, true);
    let job_id = payload(&response)["job_id"].as_str().unwrap().to_owned();
    fixture
        .service
        .engine()
        .run_job_strict(&job_id, "mcp-git-worker")
        .unwrap();
    // 重新打开可信 stdio 查询上下文，实际 remote job 主体和原请求来源仍来自持久记录。
    let mut reopened = McpService::open(McpConfig {
        data_dir: fixture.temp.path().join("data"),
        profile: ToolProfile::Manage,
        ..McpConfig::default()
    })
    .unwrap();
    assert_eq!(
        reopened.engine().job_status(&job_id).unwrap().principal,
        principal
    );
    let response = reopened.handle(&Request {
        id: json!(2),
        method: "tools/call".into(),
        params: json!({"name":"diskgraph_status","arguments":{"job_id":job_id}}),
    });
    let data = payload(&response);
    assert_eq!(data["state"], "completed");
    let reader = reopened.engine().revision_reader().unwrap();
    let receipt = reader.job_publication_receipt(&job_id).unwrap().unwrap();
    assert_eq!(data["revision"], receipt.revision_id());
    assert_eq!(data["run_id"], receipt.run_id());
    assert_eq!(receipt.base_revision_id(), fixture.revision);
    assert_eq!(
        reader.revision(receipt.revision_id()).unwrap().snapshot_id,
        reader.revision(&fixture.revision).unwrap().snapshot_id
    );
    assert!(!response.to_string().contains("dirty\\n"));
}

#[test]
fn git_arguments_are_all_or_none_and_do_not_silently_trigger_a_scan() {
    let mut fixture = GitFixture::new();
    let count = fixture.job_count();
    for missing in ["collector", "revision", "node_id"] {
        let mut arguments = fixture.arguments();
        arguments.as_object_mut().unwrap().remove(missing);
        let response = call(&mut fixture.service, arguments);
        assert_eq!(response["error"]["code"], -32602, "{missing}: {response}");
    }
    assert_eq!(fixture.job_count(), count);
}

#[test]
fn failed_git_job_exposes_only_fixed_durable_diagnostic_after_reconnect() {
    let mut fixture = GitFixture::new();
    let (response, principal) = fixture.remote_call(fixture.arguments(), ALL, true);
    let job_id = payload(&response)["job_id"].as_str().unwrap().to_owned();
    fixture
        .service
        .engine()
        .set_content_read(&fixture.scope, &principal, false)
        .unwrap();
    assert!(
        fixture
            .service
            .engine()
            .run_job_strict(&job_id, "denied-worker")
            .is_err()
    );
    let mut reopened = McpService::open(McpConfig {
        data_dir: fixture.temp.path().join("data"),
        profile: ToolProfile::Manage,
        ..McpConfig::default()
    })
    .unwrap();
    let response = reopened.handle(&Request {
        id: json!(3),
        method: "tools/call".into(),
        params: json!({"name":"diskgraph_status","arguments":{"job_id":job_id}}),
    });
    let data = payload(&response);
    assert_eq!(data["state"], "failed");
    assert_eq!(
        data["failure"],
        json!({"phase":"admission","code":"conflict"})
    );
    assert!(data.get("revision").is_none());
    assert_eq!(
        reopened
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(fixture.revision.as_str())
    );
    assert!(
        !response
            .to_string()
            .contains(fixture.temp.path().to_str().unwrap())
    );
}

#[test]
fn merged_git_sync_reports_the_existing_running_job_state() {
    let mut fixture = GitFixture::new();
    let actor = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
    fixture
        .service
        .engine()
        .set_content_read(&fixture.scope, &actor, true)
        .unwrap();
    let arguments = fixture.arguments();
    let first = call(&mut fixture.service, arguments.clone());
    let job_id = payload(&first)["job_id"].as_str().unwrap().to_owned();
    let count = fixture.job_count();
    let claimed = fixture
        .service
        .engine()
        .control_store()
        .unwrap()
        .claim_job_once_strict(&job_id, "held-merge-owner")
        .unwrap();
    assert_eq!(claimed.state, diskgraph_store::JobState::Running);
    let repeated = call(&mut fixture.service, arguments);
    assert_eq!(payload(&repeated)["job_id"], job_id);
    assert_eq!(payload(&repeated)["state"], "running");
    assert_eq!(fixture.job_count(), count);
}
