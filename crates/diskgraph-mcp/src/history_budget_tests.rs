//! D24：真实 socket 上的历史期限、双侧归属及编码后实时撤权回归。

use crate::auth::{AuthConfig, Authenticator, TokenClaims, TokenMinter};
use crate::http::{self, HttpLimits, NetworkPolicy, Security};
use crate::{McpConfig, McpService};
use diskgraph_core::{Grant, Permission, PrincipalId, QueryBudget, ScopeId};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ORIGIN: &str = "https://diskgraph-history-test.invalid";
type ReplyHook = Box<dyn FnOnce(&McpService)>;
type ThreadReplyHook = Box<dyn FnOnce(&McpService) + Send>;
type SocketHooks = (Option<Sender<()>>, Option<ThreadReplyHook>);

thread_local! {
    static STARTED: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
    static BEFORE_REPLY: RefCell<Option<ReplyHook>> = const { RefCell::new(None) };
}

pub(super) fn request_started() {
    if let Some(sender) = STARTED.with(|slot| slot.borrow_mut().take()) {
        sender.send(()).unwrap();
    }
}

pub(super) fn before_reply(service: &McpService) {
    if let Some(hook) = BEFORE_REPLY.with(|slot| slot.borrow_mut().take()) {
        hook(service);
    }
}

/// 原生隔离数据库夹具；本地管理员仅负责准备，所有断言通过认证远程服务。
struct Fixture {
    _directory: tempfile::TempDir,
    service: McpService,
    security: Security,
    token: String,
    empty_token: String,
    principal: PrincipalId,
    control_path: PathBuf,
    scopes: [ScopeId; 3],
    revisions: [String; 3],
}

impl Fixture {
    fn new(label: &str) -> Self {
        let (mut local, directory) =
            crate::tests::service(crate::protocol::ToolProfile::ReadFull, label);
        let scopes = ["before", "after", "remaining"].map(|name| {
            let root = directory.path().join(name);
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("ordinary.txt"), name).unwrap();
            ScopeId::new(crate::tests::seed(&mut local, &root)).unwrap()
        });
        let revisions = scopes
            .each_ref()
            .map(|scope| local.engine.latest_revision(scope).unwrap().unwrap());
        let authenticator = Authenticator::new(AuthConfig::single(
            "history-test-issuer",
            "history-test-audience",
            b"history-test-key",
        ));
        let mint = |scope| {
            TokenMinter::new(b"history-test-key").mint(&TokenClaims {
                issuer: "history-test-issuer".into(),
                audience: "history-test-audience".into(),
                subject: "history-reader".into(),
                expires_at_unix_seconds: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 300,
                scope,
            })
        };
        let token = mint(Some("metadata:read".into()));
        let empty_token = mint(None);
        let principal = authenticator.authenticate(Some(&token)).unwrap().principal;
        let control_path = directory.path().join("data/diskgraph-control.sqlite");
        {
            let mut control = diskgraph_store::ControlStore::open(&control_path).unwrap();
            let policy_version = control.policy_version().unwrap();
            assert!(
                policy_version > 0,
                "remote fixture must use persistent policy"
            );
            for scope in &scopes {
                control
                    .upsert_grant(&Grant {
                        principal: principal.clone(),
                        permission: Permission::MetadataRead,
                        scope: scope.clone(),
                        policy_version,
                    })
                    .unwrap();
            }
        }
        let service = McpService::open_remote(McpConfig {
            data_dir: directory.path().join("data"),
            ..McpConfig::default()
        })
        .unwrap();
        let security = Security {
            authenticator: Some(Arc::new(authenticator)),
            policy: NetworkPolicy::default().with_origins([ORIGIN.to_owned()]),
        };
        Self {
            _directory: directory,
            service,
            security,
            token,
            empty_token,
            principal,
            control_path,
            scopes,
            revisions,
        }
    }

    fn arguments(&self) -> Value {
        // 不传 scope 提示：两个 revision 必须分别按真实持久归属授权。
        json!({"before":self.revisions[0], "after":self.revisions[1]})
    }

    fn call(&self, tool: &str, arguments: Value) -> (u16, Value) {
        finish_socket(start_socket(
            self.service.clone(),
            self.security.clone(),
            &self.token,
            ORIGIN,
            tool,
            arguments,
            (None, None),
        ))
    }
}

#[test]
fn history_decode_failure_keeps_a_bounded_business_diagnostic_on_the_socket() {
    let fixture = Fixture::new("history-error-bytes");
    let arguments = json!({"before": fixture.revisions[0], "after": fixture.revisions[0]});
    let (status, good) = fixture.call("diskgraph_changes", arguments.clone());
    assert_eq!(status, 200);
    assert!(
        good.get("result").is_some(),
        "positive history failed: {good}"
    );
    rusqlite::Connection::open(
        fixture
            .control_path
            .parent()
            .unwrap()
            .join("diskgraph.sqlite"),
    )
    .unwrap()
    .execute(
        "UPDATE nodes SET kind=?1 WHERE parent_id IS NULL",
        ["\0".repeat(11_000)],
    )
    .unwrap();
    let (status, bad) = fixture.call("diskgraph_changes", arguments);
    assert_eq!(status, 200);
    assert_eq!(bad["error"]["data"]["business_code"], "internal_error");
    assert_eq!(bad["error"]["data"]["exit_code"], 10);
    assert!(bad.get("result").is_none());
    let bytes = serde_json::to_vec(&bad).unwrap().len();
    assert!(
        bytes <= QueryBudget::default().max_response_bytes,
        "encoded socket diagnostic={bytes} cap={}",
        QueryBudget::default().max_response_bytes
    );
}

/// 一次实际 TCP 请求；只运行一个 accepted connection，线程始终由调用者 join。
fn start_socket(
    mut service: McpService,
    security: Security,
    token: &str,
    origin: &str,
    tool: &str,
    arguments: Value,
    hooks: SocketHooks,
) -> (TcpStream, JoinHandle<()>) {
    let (started, hook) = hooks;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        STARTED.with(|slot| *slot.borrow_mut() = started);
        BEFORE_REPLY.with(|slot| *slot.borrow_mut() = hook.map(|hook| hook as ReplyHook));
        let limits = HttpLimits::default();
        let request = http::read_request(&mut BufReader::new(stream.try_clone().unwrap()), &limits)
            .unwrap()
            .unwrap();
        let response = http::handle_secured(&mut service, &request, &limits, &security);
        http::write_response(&mut stream, &response).unwrap();
        STARTED.with(|slot| slot.borrow_mut().take());
        BEFORE_REPLY.with(|slot| slot.borrow_mut().take());
        stream.shutdown(Shutdown::Write).unwrap();
    });
    let body = json!({"jsonrpc":"2.0", "id":41, "method":"tools/call",
        "params":{"name":tool, "arguments":arguments}})
    .to_string();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "POST /mcp HTTP/1.1\r\nHost: localhost\r\nOrigin: {origin}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    (stream, worker)
}

fn finish_socket((mut stream, worker): (TcpStream, JoinHandle<()>)) -> (u16, Value) {
    let mut bytes = Vec::new();
    let read = stream.read_to_end(&mut bytes);
    worker.join().unwrap();
    read.unwrap();
    let split = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let header = std::str::from_utf8(&bytes[..split]).unwrap();
    let status = header.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = serde_json::from_slice(&bytes[split + 4..]).unwrap();
    (status, body)
}

fn permission_denied(status: u16, reply: &Value) {
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["error"]["code"], -32001, "{reply}");
    assert_eq!(
        reply["error"]["data"]["business_code"], "permission_denied",
        "{reply}"
    );
    assert_eq!(reply["error"]["data"]["exit_code"], 3, "{reply}");
    assert!(reply.get("result").is_none(), "denial leaked data: {reply}");
}

#[test]
fn history_and_growth_use_real_authentication_origin_and_revision_ownership() {
    let fixture = Fixture::new("history-real-socket");
    for tool in ["diskgraph_changes", "diskgraph_growth"] {
        let (status, response) = fixture.call(tool, fixture.arguments());
        assert_eq!(status, 200);
        assert_eq!(response["result"]["isError"], false, "{response}");
        assert_eq!(response["result"]["structuredContent"]["truncated"], false);
        let (status, normal) = fixture.call(
            tool,
            json!({
                "scope":fixture.scopes[0].as_str(),
                "before":fixture.revisions[0], "after":fixture.revisions[0],
            }),
        );
        assert_eq!(status, 200);
        assert_eq!(normal["result"]["isError"], false, "{normal}");
        let mut mismatched = fixture.arguments();
        mismatched["scope"] = json!(fixture.scopes[0].as_str());
        let (status, denied) = fixture.call(tool, mismatched);
        permission_denied(status, &denied);
        let (status, denied) = finish_socket(start_socket(
            fixture.service.clone(),
            fixture.security.clone(),
            &fixture.empty_token,
            ORIGIN,
            tool,
            fixture.arguments(),
            (None, None),
        ));
        permission_denied(status, &denied);
    }
    let (status, refused) = finish_socket(start_socket(
        fixture.service.clone(),
        fixture.security.clone(),
        &fixture.token,
        "https://untrusted-history-test.invalid",
        "diskgraph_changes",
        fixture.arguments(),
        (None, None),
    ));
    assert_eq!(status, 403);
    assert_eq!(refused["error"], "forbidden_origin");
}

fn expired_initial_authorization(tool: &str) {
    let fixture = Fixture::new("history-initial-wait");
    let (normal_status, normal) = fixture.call(tool, fixture.arguments());
    assert_eq!(normal_status, 200);
    assert_eq!(normal["result"]["isError"], false, "{normal}");
    let guard = fixture.service.engine.control_store().unwrap();
    let (sender, receiver) = channel();
    let socket = start_socket(
        fixture.service.clone(),
        fixture.security.clone(),
        &fixture.token,
        ORIGIN,
        tool,
        fixture.arguments(),
        (Some(sender), None),
    );
    let started = receiver.recv_timeout(Duration::from_secs(5));
    // 截止时刻已经在真实 tools/call 执行线程生成；保持同 Engine 控制锁超过它。
    if started.is_ok() {
        std::thread::sleep(Duration::from_millis(
            QueryBudget::default().deadline_ms + 100,
        ));
    }
    drop(guard);
    let (status, reply) = finish_socket(socket);
    started.unwrap();
    assert_eq!(status, 200, "{reply}");
    assert!(
        reply.get("error").is_some()
            || (reply["result"]["isError"] == false
                && reply["result"]["structuredContent"]["truncated"] == true),
        "authorization wait returned a late complete {tool} response: {reply}",
    );
    if reply.get("error").is_some() {
        assert!(
            matches!(
                reply["error"]["data"]["business_code"].as_str(),
                Some("budget_exceeded" | "timeout")
            ),
            "deadline must remain a business budget error: {reply}"
        );
        assert_eq!(reply["error"]["data"]["exit_code"], 7);
        assert!(reply.get("result").is_none());
    }
}

#[test]
fn socket_history_initial_authorization_cannot_reset_deadline() {
    expired_initial_authorization("diskgraph_changes");
}

#[test]
fn socket_growth_initial_authorization_cannot_reset_deadline() {
    expired_initial_authorization("diskgraph_growth");
}

fn terminal_revocation(tool: &str, revoke_scope: bool) {
    for side in 0..2 {
        let fixture = Fixture::new("history-terminal-auth");
        let path = fixture.control_path.clone();
        let scope = fixture.scopes[side].clone();
        let principal = fixture.principal.clone();
        let observed = Arc::new(AtomicBool::new(false));
        let callback_observed = observed.clone();
        let hook: ThreadReplyHook = Box::new(move |service| {
            // handle_secured 的真实 bearer 主体必须进入编码后的观察点。
            assert_eq!(service.context.principal(), &principal);
            assert!(!service.context.trusted_local());
            let mut control = diskgraph_store::ControlStore::open(&path).unwrap();
            if revoke_scope {
                control.revoke_scope(&scope).unwrap();
            } else {
                control
                    .revoke_grant(&principal, &Permission::MetadataRead, &scope)
                    .unwrap();
            }
            callback_observed.store(true, Ordering::SeqCst);
        });
        let (status, reply) = finish_socket(start_socket(
            fixture.service.clone(),
            fixture.security.clone(),
            &fixture.token,
            ORIGIN,
            tool,
            fixture.arguments(),
            (None, Some(hook)),
        ));
        assert!(
            observed.load(Ordering::SeqCst),
            "actual encoded reply hook was not reached"
        );
        let control = diskgraph_store::ControlStore::open(&fixture.control_path).unwrap();
        assert_eq!(
            control
                .live_permission(
                    &fixture.principal,
                    &Permission::MetadataRead,
                    &fixture.scopes[2],
                )
                .unwrap(),
            Some(true),
            "single revocation removed unrelated authorization"
        );
        let (remaining_status, remaining) = fixture.call(
            "diskgraph_node",
            json!({
                "scope":fixture.scopes[2].as_str(), "revision":fixture.revisions[2],
            }),
        );
        assert_eq!(remaining_status, 200);
        assert_eq!(remaining["result"]["isError"], false, "{remaining}");
        permission_denied(status, &reply);
    }
}

#[test]
fn encoded_socket_history_rechecks_both_scope_sides() {
    terminal_revocation("diskgraph_changes", true);
}

#[test]
fn encoded_socket_history_rechecks_both_grant_sides() {
    terminal_revocation("diskgraph_changes", false);
}

#[test]
fn encoded_socket_growth_rechecks_both_scope_sides() {
    terminal_revocation("diskgraph_growth", true);
}

#[test]
fn encoded_socket_growth_rechecks_both_grant_sides() {
    terminal_revocation("diskgraph_growth", false);
}
