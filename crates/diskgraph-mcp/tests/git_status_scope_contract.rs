//! Git C04 的真实任务 scope、权限和回执 envelope；来源：签名认证请求及真实 Git/扫描。
use diskgraph_core::{Grant, Permission, PrincipalId, ScopeId};
use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits, HttpRequest};
use diskgraph_mcp::protocol::ToolProfile;
use diskgraph_mcp::{McpConfig, McpService, STDIO_PRINCIPAL};
use diskgraph_store::JobState;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ALL: &str = "metadata:read index:write content:read operations:view";

/// 两个真实注册范围，B 为当前默认范围，A 为显式 Git 任务目标；无伪造 ownership。
/// 来源：原生 Rust MCP 签名请求与 Engine 实际 Git/扫描集成夹具。
struct StatusFixture {
    _temp: tempfile::TempDir,
    service: McpService,
    actor: PrincipalId,
    a: ScopeId,
    b: ScopeId,
    base: String,
    node: u64,
}
impl StatusFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let a_root = temp.path().join("repo-a");
        let b_root = temp.path().join("scope-b");
        std::fs::create_dir(&a_root).unwrap();
        std::fs::create_dir(&b_root).unwrap();
        std::fs::write(temp.path().join("empty-config"), "").unwrap();
        git(&a_root, temp.path(), &["init", "-q"]);
        std::fs::write(a_root.join("tracked"), "committed\n").unwrap();
        git(&a_root, temp.path(), &["add", "tracked"]);
        git(
            &a_root,
            temp.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=f@example.invalid",
                "commit",
                "-qm",
                "base",
            ],
        );
        let service = McpService::open(McpConfig {
            data_dir: temp.path().join("data"),
            profile: ToolProfile::Manage,
            ..McpConfig::default()
        })
        .unwrap();
        let actor = PrincipalId::new(STDIO_PRINCIPAL).unwrap();
        let engine = service.engine();
        // 实际 Store 当前按最早注册顺序，resolve_scope 取首项；不依赖其“newest”注释。
        let b = engine
            .register_scope(&b_root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        scan(&service, &b, &actor);
        std::thread::sleep(Duration::from_millis(2));
        let a = engine
            .register_scope(&a_root, &actor, &engine.policy_authorizer().unwrap())
            .unwrap();
        let base = scan(&service, &a, &actor);
        let node = engine.revision_root_node(&base).unwrap().id;
        let remote = authenticator().authenticate(Some(&token(ALL))).unwrap();
        {
            let mut control = engine.control_store().unwrap();
            let version = control.policy_version().unwrap();
            for scope in [&a, &b] {
                for permission in [
                    Permission::MetadataRead,
                    Permission::IndexWrite,
                    Permission::ContentRead,
                    Permission::OperationView,
                ] {
                    control
                        .upsert_grant(&Grant {
                            principal: remote.principal.clone(),
                            permission,
                            scope: scope.clone(),
                            policy_version: version,
                        })
                        .unwrap();
                }
            }
        }
        assert_eq!(
            engine
                .list_scopes(&remote.principal, &engine.policy_authorizer().unwrap())
                .unwrap()[0]
                .scope_id,
            b
        );
        Self {
            _temp: temp,
            service,
            actor,
            a,
            b,
            base,
            node,
        }
    }
    fn request(&mut self, tool: &str, arguments: Value, ceiling: &str) -> Value {
        let request = HttpRequest {
            method: "POST".into(), path: "/mcp".into(), query: String::new(),
            headers: HashMap::from([("authorization".into(), format!("Bearer {}", token(ceiling)))]),
            body: json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}}).to_string(),
        };
        let response = http::handle_authenticated(
            &mut self.service,
            &request,
            &HttpLimits::default(),
            Some(&authenticator()),
        );
        serde_json::from_str(&response.body).unwrap()
    }
    fn enqueue(&mut self) -> String {
        let response = self.request("diskgraph_sync", json!({"scope":self.a.as_str(),"revision":self.base,"node_id":self.node,"collector":"git"}), ALL);
        data(&response)["job_id"].as_str().unwrap().to_owned()
    }
    fn complete(&mut self) -> (String, diskgraph_core::JobPublicationReceipt) {
        let job = self.enqueue();
        assert_eq!(
            self.service
                .engine()
                .run_job_strict(&job, "status-worker")
                .unwrap()
                .state,
            JobState::Completed
        );
        let receipt = self
            .service
            .engine()
            .revision_reader()
            .unwrap()
            .job_publication_receipt(&job)
            .unwrap()
            .unwrap();
        (job, receipt)
    }
}
fn authenticator() -> Authenticator {
    Authenticator::new(AuthConfig::single(
        "status-fixture",
        "diskgraph",
        b"isolated-status-test-key",
    ))
}
fn token(ceiling: &str) -> String {
    TokenMinter::new(b"isolated-status-test-key").mint(&TokenClaims {
        issuer: "status-fixture".into(),
        audience: "diskgraph".into(),
        subject: "observer".into(),
        expires_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 300,
        scope: Some(ceiling.into()),
    })
}
fn scan(service: &McpService, scope: &ScopeId, actor: &PrincipalId) -> String {
    let engine = service.engine();
    let job = engine
        .index_scope(scope, actor, &engine.policy_authorizer().unwrap())
        .unwrap();
    assert_eq!(
        engine.run_job(&job.job_id, "status-scan").unwrap().state,
        JobState::Completed
    );
    engine
        .revision_for_job(&job.job_id, actor, &engine.policy_authorizer().unwrap())
        .unwrap()
}
fn git(root: &Path, home: &Path, arguments: &[&str]) {
    let mut command = Command::new("git");
    command.env_clear();
    for key in ["PATH", "SystemRoot"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty-config"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture Git {arguments:?}: {output:?}"
    );
}
fn data(response: &Value) -> &Value {
    assert_eq!(response["result"]["isError"], false, "{response}");
    &response["result"]["structuredContent"]["data"]
}

#[test]
fn completed_git_status_binds_outer_scope_and_revision_to_actual_job_not_default_scope() {
    let mut f = StatusFixture::new();
    let (job, receipt) = f.complete();
    let response = f.request("diskgraph_status", json!({"job_id":job}), ALL);
    assert_eq!(data(&response)["scope_id"], f.a.as_str());
    assert_eq!(data(&response)["revision_id"], receipt.revision_id());
    let envelope = &response["result"]["structuredContent"];
    assert_eq!(
        envelope["scope_id"],
        f.a.as_str(),
        "default B cannot label an actual A job"
    );
    assert_eq!(envelope["revision_id"], receipt.revision_id());
}

#[test]
fn git_status_explicit_other_scope_is_permission_denied_even_when_both_are_authorized() {
    let mut f = StatusFixture::new();
    let (job, _) = f.complete();
    let response = f.request(
        "diskgraph_status",
        json!({"scope":f.b.as_str(),"job_id":job}),
        ALL,
    );
    assert_eq!(
        response["error"]["data"]["business_code"], "permission_denied",
        "{response}"
    );
}

#[test]
fn completed_git_status_outer_revision_does_not_follow_a_new_scan_latest() {
    let mut f = StatusFixture::new();
    let (job, receipt) = f.complete();
    let newer = scan(&f.service, &f.a, &f.actor);
    assert_ne!(newer, receipt.revision_id());
    let response = f.request(
        "diskgraph_status",
        json!({"scope":f.a.as_str(),"job_id":job}),
        ALL,
    );
    assert_eq!(data(&response)["revision_id"], receipt.revision_id());
    assert_eq!(
        response["result"]["structuredContent"]["revision_id"],
        receipt.revision_id(),
        "envelope must bind the same immutable result as data"
    );
}

#[test]
fn queued_git_status_requires_only_verified_actual_scope_operation_view() {
    let mut f = StatusFixture::new();
    let job = f.enqueue();
    assert_eq!(
        f.service.engine().job_status(&job).unwrap().state,
        JobState::Queued
    );
    let response = f.request(
        "diskgraph_status",
        json!({"scope":f.a.as_str(),"job_id":job}),
        "operations:view",
    );
    assert_eq!(data(&response)["state"], "queued");
    assert_eq!(
        response["result"]["structuredContent"]["scope_id"],
        f.a.as_str()
    );
    assert!(
        response["result"]["structuredContent"]["revision_id"].is_null(),
        "queued observation has no published result"
    );
}
