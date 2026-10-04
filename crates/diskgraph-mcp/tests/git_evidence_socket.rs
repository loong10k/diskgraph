//! 真实签名、TCP 分发与持久 Git 任务的验收；来源：原生 Rust SC-06 / EC-02。

use diskgraph_core::{Grant, Permission, PrincipalId, ScopeId};
use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits};
use diskgraph_mcp::protocol::ToolProfile;
use diskgraph_mcp::{McpConfig, McpService};
use diskgraph_store::{JobRecord, JobState};
use serde_json::{Value, json};
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ISSUER: &str = "git-job-socket-fixture";
const AUDIENCE: &str = "diskgraph-test";
const KEY: &[u8] = b"isolated-git-socket-fixture-key";
const CAPABILITIES: &str = "index:write metadata:read content:read operations:view";

/// 隔离真实仓库、索引、双主体授权及实际认证 TCP 入口。
/// 来源：DiskGraph 原生 Rust 集成夹具，无 Java 对应对象。
struct GitSocketFixture {
    directory: tempfile::TempDir,
    service: McpService,
    authenticator: Authenticator,
    scope: ScopeId,
    writer: PrincipalId,
    other: PrincipalId,
    revision: String,
    node_id: u64,
}

impl GitSocketFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(directory.path().join("empty-config"), b"").unwrap();
        git(&root, directory.path(), &["init", "-q"]);
        std::fs::write(root.join("tracked.txt"), b"original\n").unwrap();
        git(&root, directory.path(), &["add", "tracked.txt"]);
        git(
            &root,
            directory.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=f@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        std::fs::write(root.join("tracked.txt"), b"private dirty source body\n").unwrap();
        let service = McpService::open_remote(McpConfig {
            data_dir: directory.path().join("data"),
            profile: ToolProfile::Manage,
            ..McpConfig::default()
        })
        .unwrap();
        let admin = PrincipalId::new("git-socket-fixture-admin").unwrap();
        service.engine().bootstrap_local_admin(&admin).unwrap();
        let authorizer = service.engine().policy_authorizer().unwrap();
        let scope = service
            .engine()
            .register_scope(&root, &admin, &authorizer)
            .unwrap();
        // 注册范围写入了新授权；重新加载策略，不能拿注册前的快照执行索引。
        let authorizer = service.engine().policy_authorizer().unwrap();
        let index = service
            .engine()
            .index_scope(&scope, &admin, &authorizer)
            .unwrap();
        assert_eq!(
            service
                .engine()
                .run_job(&index.job_id, "fixture-index")
                .unwrap()
                .state,
            JobState::Completed
        );
        let revision = service
            .engine()
            .revision_for_job(&index.job_id, &admin, &authorizer)
            .unwrap();
        let reader = service.engine().revision_reader().unwrap();
        let snapshot = reader.revision(&revision).unwrap().snapshot_id;
        let node_id = reader.load(&snapshot).unwrap().root().id;
        let mut config = AuthConfig::single(ISSUER, AUDIENCE, KEY);
        config.clock_skew_seconds = 0;
        let authenticator = Authenticator::new(config);
        let principals = ["writer", "other"].map(|subject| {
            authenticator
                .authenticate(Some(&mint(subject, now() + 300, CAPABILITIES)))
                .unwrap()
                .principal
        });
        {
            let mut control = service.engine().control_store().unwrap();
            let policy_version = control.policy_version().unwrap();
            for principal in &principals {
                for permission in [
                    Permission::IndexWrite,
                    Permission::MetadataRead,
                    Permission::ContentRead,
                    Permission::OperationView,
                ] {
                    control
                        .upsert_grant(&Grant {
                            principal: principal.clone(),
                            permission,
                            scope: scope.clone(),
                            policy_version,
                        })
                        .unwrap();
                }
            }
        }
        assert_ne!(principals[0], principals[1]);
        Self {
            directory,
            service,
            authenticator,
            scope,
            writer: principals[0].clone(),
            other: principals[1].clone(),
            revision,
            node_id,
        }
    }

    fn target(&self) -> Value {
        json!({"scope": self.scope.as_str(), "collector": "git", "revision": self.revision, "node_id": self.node_id})
    }

    fn call(&self, token: &str, tool: &str, arguments: Value) -> (u16, Value) {
        let body = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":tool,"arguments":arguments}}).to_string();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut service = self.service.clone();
        let authenticator = self.authenticator.clone();
        // 真实单次 TCP handler，无替代认证、无自动后台 runner；失败断言前完成 join。
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let limits = HttpLimits {
                read_timeout: Duration::from_secs(5),
                ..HttpLimits::default()
            };
            let request =
                http::read_request(&mut BufReader::new(stream.try_clone().unwrap()), &limits)
                    .unwrap()
                    .unwrap();
            let response =
                http::handle_authenticated(&mut service, &request, &limits, Some(&authenticator));
            http::write_response(&mut stream, &response).unwrap();
            stream.shutdown(Shutdown::Both).unwrap();
        });
        write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        stream.flush().unwrap();
        let mut bytes = Vec::new();
        let read_result = (&mut stream).take(65_536).read_to_end(&mut bytes);
        let joined = worker.join();
        read_result.unwrap();
        joined.unwrap();
        let response = String::from_utf8(bytes).unwrap();
        let (header, body) = response.split_once("\r\n\r\n").unwrap();
        (
            header.split_whitespace().nth(1).unwrap().parse().unwrap(),
            serde_json::from_str(body).unwrap(),
        )
    }

    fn enqueue(&self, token: &str) -> JobRecord {
        let (status, response) = self.call(token, "diskgraph_sync", self.target());
        assert_eq!(status, 200, "{response}");
        assert_eq!(response["result"]["isError"], false, "{response}");
        let job = self
            .service
            .engine()
            .job_status(data(&response)["job_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(serde_json::to_value(job.kind).unwrap(), "git_evidence");
        assert_eq!(job.principal, self.writer);
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(
            self.service
                .engine()
                .latest_revision(&self.scope)
                .unwrap()
                .as_deref(),
            Some(self.revision.as_str())
        );
        job
    }

    fn run(&self, job: &JobRecord) -> JobRecord {
        let runner = self.service.start_job_runner();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let observed = self.service.engine().job_status(&job.job_id).unwrap();
            if !matches!(observed.state, JobState::Queued | JobState::Running)
                || Instant::now() >= deadline
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        runner.stop();
        self.service.engine().job_status(&job.job_id).unwrap()
    }

    fn job_count(&self) -> i64 {
        rusqlite::Connection::open(self.directory.path().join("data/diskgraph-control.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn mint(subject: &str, expiry: u64, capabilities: &str) -> String {
    TokenMinter::new(KEY).mint(&TokenClaims {
        issuer: ISSUER.into(),
        audience: AUDIENCE.into(),
        subject: subject.into(),
        expires_at_unix_seconds: expiry,
        scope: Some(capabilities.into()),
    })
}
fn data(response: &Value) -> &Value {
    &response["result"]["structuredContent"]["data"]
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
        "Git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn real_socket_git_job_intersects_verified_capabilities_and_publishes_safe_result() {
    let fixture = GitSocketFixture::new();
    let before = fixture.job_count();
    let token = mint("writer", now() + 300, CAPABILITIES);
    let job = fixture.enqueue(&token);
    assert_eq!(fixture.job_count(), before + 1);
    assert!(
        fixture
            .service
            .engine()
            .control_store()
            .unwrap()
            .live_permission(&fixture.other, &Permission::ContentRead, &fixture.scope)
            .unwrap()
            .unwrap()
    );
    for token in [
        mint(
            "other",
            now() + 300,
            "index:write metadata:read operations:view",
        ),
        mint("outsider", now() + 300, CAPABILITIES),
    ] {
        let (status, rejected) = fixture.call(&token, "diskgraph_sync", fixture.target());
        assert_eq!(status, 200, "{rejected}");
        assert_eq!(rejected["error"]["code"], -32001, "{rejected}");
        assert_eq!(
            rejected["error"]["data"]["business_code"], "permission_denied",
            "{rejected}"
        );
        assert_eq!(fixture.job_count(), before + 1);
    }
    let persisted = fixture
        .service
        .engine()
        .control_store()
        .unwrap()
        .job_request_authority(&job.job_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.principal(), &fixture.writer);
    assert!(!serde_json::to_string(&persisted).unwrap().contains(&token));
    assert_eq!(fixture.run(&job).state, JobState::Completed);
    let (status, result) = fixture.call(&token, "diskgraph_status", json!({"job_id":job.job_id}));
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["result"]["isError"], false, "{result}");
    assert_eq!(data(&result)["state"], "completed");
    let actual = fixture
        .service
        .engine()
        .latest_revision(&fixture.scope)
        .unwrap()
        .unwrap();
    assert_ne!(actual, fixture.revision);
    assert_eq!(data(&result)["revision_id"], actual);
    assert!(data(&result)["run_id"].as_str().is_some());
    assert!(!result.to_string().contains("private dirty source body"));
    assert!(!result.to_string().contains(&token));
}

#[test]
fn real_socket_identical_git_targets_never_merge_across_verified_principals() {
    let fixture = GitSocketFixture::new();
    let before = fixture.job_count();
    let first = fixture.enqueue(&mint("writer", now() + 300, CAPABILITIES));
    let (status, response) = fixture.call(
        &mint("other", now() + 300, CAPABILITIES),
        "diskgraph_sync",
        fixture.target(),
    );
    assert_eq!(status, 200, "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    let second = fixture
        .service
        .engine()
        .job_status(data(&response)["job_id"].as_str().unwrap())
        .unwrap();
    assert_ne!(first.job_id, second.job_id);
    assert_eq!(first.principal, fixture.writer);
    assert_eq!(second.principal, fixture.other);
    assert_eq!(second.state, JobState::Queued);
    assert_eq!(fixture.job_count(), before + 2);
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
fn real_socket_queued_git_expiry_cannot_be_extended_by_reconnecting() {
    let fixture = GitSocketFixture::new();
    let expiry = now() + 10;
    let original = mint("writer", expiry, CAPABILITIES);
    let job = fixture.enqueue(&original);
    assert!(now() < expiry, "actual admission missed original expiry");
    let deadline = Instant::now() + Duration::from_secs(12);
    while now() < expiry {
        assert!(
            Instant::now() < deadline,
            "system clock did not advance to expiry"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let (status, rejected) =
        fixture.call(&original, "diskgraph_status", json!({"job_id":job.job_id}));
    assert_eq!(status, 401, "{rejected}");
    assert_eq!(rejected["reason"], "expired_token");
    let renewed = mint("writer", now() + 300, CAPABILITIES);
    let (_, queued) = fixture.call(&renewed, "diskgraph_status", json!({"job_id":job.job_id}));
    assert_eq!(data(&queued)["state"], "queued");
    assert_eq!(fixture.run(&job).state, JobState::Failed);
    assert_eq!(
        fixture
            .service
            .engine()
            .latest_revision(&fixture.scope)
            .unwrap()
            .as_deref(),
        Some(fixture.revision.as_str())
    );
    let (_, failed) = fixture.call(&renewed, "diskgraph_status", json!({"job_id":job.job_id}));
    assert_eq!(data(&failed)["state"], "failed");
    assert_eq!(data(&failed)["failure"]["phase"], "admission");
    assert!(data(&failed)["revision_id"].is_null());
    assert!(data(&failed)["run_id"].is_null());
}

#[test]
fn real_socket_queued_git_status_needs_only_actual_scope_operation_view() {
    let fixture = GitSocketFixture::new();
    let job = fixture.enqueue(&mint("writer", now() + 300, CAPABILITIES));
    let view = mint("writer", now() + 300, "operations:view");
    let (status, response) = fixture.call(&view, "diskgraph_status", json!({"job_id":job.job_id}));
    assert_eq!(status, 200, "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    let envelope = &response["result"]["structuredContent"];
    assert_eq!(envelope["scope_id"], fixture.scope.as_str());
    assert_eq!(data(&response)["scope_id"], fixture.scope.as_str());
    assert_eq!(data(&response)["state"], "queued");
    assert!(envelope["revision_id"].is_null());
    assert!(data(&response)["revision_id"].is_null());
    assert_eq!(fixture.job_count(), 2);
    // 仅状态权限不能被状态查询补成内容或写能力，实际任务也没有被隐式执行。
    let (_, denied) = fixture.call(&view, "diskgraph_sync", fixture.target());
    assert_eq!(
        denied["error"]["data"]["business_code"],
        "permission_denied"
    );
    assert_eq!(fixture.job_count(), 2);
    assert_eq!(
        fixture
            .service
            .engine()
            .job_status(&job.job_id)
            .unwrap()
            .state,
        JobState::Queued
    );
}
