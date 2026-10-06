#[path = "support/native_scan_service.rs"]
mod native_scan_service;
use diskgraph_core::{Grant, Permission, PrincipalId, ScopeId};
use diskgraph_mcp::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use diskgraph_mcp::http::{self, HttpLimits};
use diskgraph_mcp::protocol::ToolProfile;
use diskgraph_mcp::{McpConfig, McpService};
use diskgraph_store::{JobKind, JobRecord, JobState};
use serde_json::{Value, json};
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ISSUER: &str = "durable-job-test-issuer";
const AUDIENCE: &str = "diskgraph-test";
const SIGNING_KEY: &[u8] = b"isolated-durable-job-test-key";
const WRITER_CAPABILITIES: &str = "index:write metadata:read operations:view";

/// 隔离真实根、控制库和图库，使用两个经实际签名验证的远程主体。
/// 来源：DiskGraph 原生 Rust SC-06 持久请求授权回归，无 Java 对应对象。
struct DurableFixture {
    _scan_recovery: native_scan_service::NativeScanRecovery,
    _directory: tempfile::TempDir,
    service: McpService,
    authenticator: Authenticator,
    scope: ScopeId,
    writer: PrincipalId,
    other: PrincipalId,
}

impl DurableFixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("scope");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("visible.txt"), b"isolated scan fixture").unwrap();
        let (service, _scan_recovery) = native_scan_service::open(
            McpConfig {
                data_dir: directory.path().join("data"),
                profile: ToolProfile::Manage,
                ..McpConfig::default()
            },
            true,
        )
        .unwrap();
        let administrator = PrincipalId::new("durable-fixture-admin").unwrap();
        service
            .engine()
            .bootstrap_local_admin(&administrator)
            .unwrap();
        let scope = service
            .engine()
            .register_scope(
                &root,
                &administrator,
                &service.engine().policy_authorizer().unwrap(),
            )
            .unwrap();
        let mut config = AuthConfig::single(ISSUER, AUDIENCE, SIGNING_KEY);
        // 不改变生产时钟；HTTP 和任务均使用签名里的同一真实绝对到期值。
        config.clock_skew_seconds = 0;
        let authenticator = Authenticator::new(config);
        let principals = ["writer", "other"].map(|subject| {
            authenticator
                .authenticate(Some(&mint(
                    subject,
                    unix_seconds() + 300,
                    WRITER_CAPABILITIES,
                )))
                .unwrap()
                .principal
        });
        {
            let mut control = service.engine().control_store().unwrap();
            let version = control.policy_version().unwrap();
            for principal in &principals {
                for permission in [
                    Permission::IndexWrite,
                    Permission::MetadataRead,
                    Permission::OperationView,
                ] {
                    control
                        .upsert_grant(&Grant {
                            principal: principal.clone(),
                            permission,
                            scope: scope.clone(),
                            policy_version: version,
                        })
                        .unwrap();
                }
            }
        }
        assert_ne!(principals[0], principals[1]);
        Self {
            _scan_recovery,
            _directory: directory,
            service,
            authenticator,
            scope,
            writer: principals[0].clone(),
            other: principals[1].clone(),
        }
    }

    fn call(&self, token: &str, tool: &str, arguments: Value) -> (u16, Value) {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}
        })
        .to_string();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut service = self.service.clone();
        let authenticator = self.authenticator.clone();
        // 先连入已监听的内核队列再启动接收线程，连接失败时不会留下等待 accept 的线程。
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        // 真实单请求 TCP handler：不启动自动 runner，不替换认证或协议分发算法。
        // accept/连接只有一轮，线程随后 join；这不宣称覆盖完整 serve loop 或 SSE。
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let limits = HttpLimits {
                read_timeout: Duration::from_secs(3),
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
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream.flush().unwrap();
        let mut bytes = Vec::new();
        let read_result = (&mut stream).take(65_536).read_to_end(&mut bytes);
        let worker_result = worker.join();
        read_result.unwrap();
        worker_result.unwrap();
        let response = String::from_utf8(bytes).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    fn enqueue(&self, tool: &str, token: &str, kind: JobKind) -> JobRecord {
        let (status, response) = self.call(token, tool, json!({"scope": self.scope.as_str()}));
        assert_eq!(status, 200, "{response}");
        assert_eq!(response["result"]["isError"], false, "{response}");
        let job_id = data(&response)["job_id"].as_str().unwrap();
        let job = self.service.engine().job_status(job_id).unwrap();
        assert_eq!(job.state, JobState::Queued);
        assert_eq!(job.principal, self.writer);
        assert_eq!(job.kind, kind);
        assert_eq!(job.scope_id, self.scope);
        assert!(
            self.service
                .engine()
                .latest_revision(&self.scope)
                .unwrap()
                .is_none()
        );
        job
    }

    fn run_to_terminal(&self, job: &JobRecord) -> JobRecord {
        let runner = self.service.start_job_runner();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed = self.service.engine().job_status(&job.job_id).unwrap();
        while matches!(observed.state, JobState::Queued | JobState::Running)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
            observed = self.service.engine().job_status(&job.job_id).unwrap();
        }
        // 所有行为断言之前停止并 join，避免失败断言把工作线程留在隔离库中。
        runner.stop();
        self.service.engine().job_status(&job.job_id).unwrap()
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn mint(subject: &str, expiry: u64, capabilities: &str) -> String {
    TokenMinter::new(SIGNING_KEY).mint(&TokenClaims {
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

fn expires_before_execution(tool: &str, kind: JobKind) {
    let fixture = DurableFixture::new();
    // 夹具准备后才签发，给实际 admission 留完整十秒；不回写认证对象或伪造时钟。
    let expiry = unix_seconds() + 10;
    let token = mint("writer", expiry, WRITER_CAPABILITIES);
    let job = fixture.enqueue(tool, &token, kind);
    assert!(
        unix_seconds() < expiry,
        "admission did not occur before expiry"
    );
    eprintln!(
        "authenticated {tool} durable queued job {} before original expiry",
        job.job_id
    );
    let wait_limit = Instant::now() + Duration::from_secs(12);
    while unix_seconds() < expiry {
        assert!(
            Instant::now() < wait_limit,
            "system clock did not reach token expiry"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fixture
            .service
            .engine()
            .job_status(&job.job_id)
            .unwrap()
            .state,
        JobState::Queued
    );
    let (expired_status, expired_response) =
        fixture.call(&token, "diskgraph_status", json!({"job_id": job.job_id}));
    assert_eq!(expired_status, 401, "{expired_response}");
    assert_eq!(
        expired_response["error"], "unauthorized",
        "{expired_response}"
    );
    assert_eq!(
        expired_response["reason"], "expired_token",
        "{expired_response}"
    );
    // 新凭据只查看已存在的任务，不能由 reconnect/status 给原任务续期。
    let renewed = mint("writer", unix_seconds() + 300, WRITER_CAPABILITIES);
    let (_, reconnected) =
        fixture.call(&renewed, "diskgraph_status", json!({"job_id": job.job_id}));
    assert_eq!(data(&reconnected)["job_id"], job.job_id);
    assert_eq!(data(&reconnected)["state"], "queued");
    let result = fixture.run_to_terminal(&job);
    let latest = fixture
        .service
        .engine()
        .latest_revision(&fixture.scope)
        .unwrap();
    eprintln!(
        "expired {tool}: actual state {:?}, actual latest {latest:?}",
        result.state
    );
    assert!(
        matches!(result.state, JobState::Failed | JobState::Cancelled) && latest.is_none(),
        "expired request executed or published: actual state {:?}, actual latest {latest:?}",
        result.state
    );
    let (_, terminal) = fixture.call(&renewed, "diskgraph_status", json!({"job_id": job.job_id}));
    assert_eq!(data(&terminal)["job_id"], job.job_id);
    assert!(matches!(
        data(&terminal)["state"].as_str(),
        Some("failed" | "cancelled")
    ));
}

#[test]
fn an_authenticated_queued_index_cannot_outlive_its_original_token() {
    expires_before_execution("diskgraph_index", JobKind::Index);
}

#[test]
fn an_authenticated_queued_sync_cannot_outlive_its_original_token() {
    expires_before_execution("diskgraph_sync", JobKind::Sync);
}

#[test]
fn valid_remote_index_and_sync_publish_and_remain_queryable_after_disconnect() {
    for (tool, kind) in [
        ("diskgraph_index", JobKind::Index),
        ("diskgraph_sync", JobKind::Sync),
    ] {
        let fixture = DurableFixture::new();
        let token = mint("writer", unix_seconds() + 300, WRITER_CAPABILITIES);
        let job = fixture.enqueue(tool, &token, kind);
        let result = fixture.run_to_terminal(&job);
        assert_eq!(result.state, JobState::Completed);
        assert!(
            fixture
                .service
                .engine()
                .latest_revision(&fixture.scope)
                .unwrap()
                .is_some()
        );
        let (_, response) = fixture.call(&token, "diskgraph_status", json!({"job_id": job.job_id}));
        assert_eq!(data(&response)["job_id"], job.job_id);
        assert_eq!(data(&response)["state"], "completed");
    }
}

#[test]
fn a_database_writer_with_a_narrow_token_cannot_queue_index_or_sync() {
    let fixture = DurableFixture::new();
    let token = mint(
        "other",
        unix_seconds() + 300,
        "metadata:read operations:view",
    );
    assert!(
        fixture
            .service
            .engine()
            .control_store()
            .unwrap()
            .live_permission(&fixture.other, &Permission::IndexWrite, &fixture.scope)
            .unwrap()
            .unwrap()
    );
    for tool in ["diskgraph_index", "diskgraph_sync"] {
        let (status, response) =
            fixture.call(&token, tool, json!({"scope": fixture.scope.as_str()}));
        assert_eq!(status, 200);
        assert_eq!(
            response["error"]["data"]["business_code"], "permission_denied",
            "{response}"
        );
        assert_eq!(
            fixture
                .service
                .engine()
                .control_store()
                .unwrap()
                .active_job_count_for_principal(&fixture.other)
                .unwrap(),
            0
        );
    }
}
