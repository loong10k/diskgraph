//! 授权等待与读取结束的期限门禁；全程使用隔离普通文件。
#![cfg(any(unix, windows))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diskgraph_core::{
    Authorizer, Decision, DenyReason, Permission, PolicyAuthorizer, PrincipalId, ScopeId,
};
use diskgraph_engine::content::{
    ConservativeProbe, InspectionRequest, InspectionStop, PlaceholderProbe,
};
use diskgraph_engine::{Engine, EngineConfig};

struct Fixture {
    engine: Arc<Engine>,
    _workspace: tempfile::TempDir,
    path: PathBuf,
    principal: PrincipalId,
    scope: ScopeId,
    policy: PolicyAuthorizer,
}

impl Fixture {
    fn open(bytes: &[u8]) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("file.bin");
        std::fs::write(&path, bytes).unwrap();
        let engine = Arc::new(
            Engine::open(EngineConfig {
                data_dir: workspace.path().join("data"),
                ..EngineConfig::default()
            })
            .unwrap(),
        );
        let principal = PrincipalId::new("content-deadline-fixture").unwrap();
        engine.bootstrap_local_admin(&principal).unwrap();
        let scope = engine
            .register_scope(&root, &principal, &engine.policy_authorizer().unwrap())
            .unwrap();
        engine.set_content_read(&scope, &principal, true).unwrap();
        let policy = engine.policy_authorizer().unwrap();
        Self {
            engine,
            _workspace: workspace,
            path,
            principal,
            scope,
            policy,
        }
    }

    fn request(&self) -> InspectionRequest<'_> {
        InspectionRequest {
            scope_id: &self.scope,
            principal: &self.principal,
            path: &self.path,
            offset: 0,
            max_bytes: 1 << 20,
            cancel: None,
            chunk_bytes: 32,
        }
    }
}

#[test]
fn expired_authorization_wait_never_reads_or_confirms_a_digest() {
    struct HoldControl {
        engine: Arc<Engine>,
        worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    }
    impl PlaceholderProbe for HoldControl {
        fn is_placeholder(&self, _: &Path) -> bool {
            let engine = self.engine.clone();
            let (ready, entered) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let _control = engine.control_store().unwrap();
                ready.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(300));
            });
            *self.worker.lock().unwrap() = Some(worker);
            entered.recv().unwrap();
            false
        }
    }
    for bytes in [&b""[..], &b"32 bytes must never start late!!!"[..]] {
        let fixture = Fixture::open(bytes);
        let probe = HoldControl {
            engine: fixture.engine.clone(),
            worker: Mutex::new(None),
        };
        let result = fixture
            .engine
            .digest_bounded_until(
                &fixture.request(),
                &probe,
                &fixture.policy,
                Instant::now() + Duration::from_millis(100),
            )
            .unwrap();
        probe.worker.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(result.stopped, Some(InspectionStop::Deadline));
        assert_eq!(
            result.bytes_digested, 0,
            "no read may start after authorization consumed the deadline"
        );
        assert!(!result.confirmed());
        assert!(result.digest_hex.is_empty());
    }
}

#[test]
fn terminal_cancellation_and_authorization_never_confirm_an_empty_digest() {
    struct TerminalPolicy {
        policy: PolicyAuthorizer,
        calls: AtomicUsize,
        cancel: Arc<AtomicBool>,
        revoke: bool,
    }
    impl Authorizer for TerminalPolicy {
        fn policy_version(&self) -> u64 {
            self.policy.policy_version()
        }
        fn decide(
            &self,
            principal: &PrincipalId,
            permission: &Permission,
            scope: &ScopeId,
        ) -> Decision {
            if self.calls.fetch_add(1, Ordering::SeqCst) >= 2 {
                if self.revoke {
                    return Decision::Denied(DenyReason::NoMatchingGrant);
                }
                self.cancel.store(true, Ordering::SeqCst);
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for revoke in [false, true] {
        let fixture = Fixture::open(b"");
        let cancel = Arc::new(AtomicBool::new(false));
        let authorizer = TerminalPolicy {
            policy: fixture.policy.clone(),
            calls: AtomicUsize::new(0),
            cancel: cancel.clone(),
            revoke,
        };
        let request = InspectionRequest {
            cancel: Some(&cancel),
            ..fixture.request()
        };
        let result = fixture
            .engine
            .digest_bounded_until(
                &request,
                &ConservativeProbe,
                &authorizer,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(
            result.stopped,
            Some(if revoke {
                InspectionStop::PermissionRevoked
            } else {
                InspectionStop::Cancelled
            })
        );
        assert!(authorizer.calls.load(Ordering::SeqCst) >= 3);
        assert!(!result.confirmed());
        assert!(result.digest_hex.is_empty());
    }
}

#[test]
fn terminal_scope_revocation_also_denies_trusted_compatibility_without_a_policy() {
    struct Allow;
    impl Authorizer for Allow {
        fn decide(&self, _: &PrincipalId, _: &Permission, _: &ScopeId) -> Decision {
            Decision::Allowed
        }
    }
    struct RevokeAtTerminal {
        control_path: PathBuf,
        calls: AtomicUsize,
    }
    impl Authorizer for RevokeAtTerminal {
        fn decide(&self, _: &PrincipalId, _: &Permission, scope: &ScopeId) -> Decision {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 2 {
                // 第二隔离连接模拟真正的数据库撤销；不递归获取 Engine 的控制锁。
                let control = rusqlite::Connection::open(&self.control_path).unwrap();
                assert_eq!(
                    control
                        .execute(
                            "UPDATE scopes SET revoked=1 WHERE scope_id=?1",
                            [scope.as_str()]
                        )
                        .unwrap(),
                    1
                );
            }
            Decision::Allowed
        }
    }
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let path = root.join("empty.bin");
    std::fs::write(&path, []).unwrap();
    let data_dir = workspace.path().join("data");
    let engine = Engine::open(EngineConfig {
        data_dir: data_dir.clone(),
        ..EngineConfig::default()
    })
    .unwrap();
    let principal = PrincipalId::new("trusted-compatibility-fixture").unwrap();
    let scope = engine.register_scope(&root, &principal, &Allow).unwrap();
    assert!(
        engine
            .control_store()
            .unwrap()
            .policy_state()
            .unwrap()
            .is_none()
    );
    let authorizer = RevokeAtTerminal {
        control_path: data_dir.join("diskgraph-control.sqlite"),
        calls: AtomicUsize::new(0),
    };
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 100,
        chunk_bytes: 32,
        cancel: None,
    };
    let result = engine
        .digest_bounded_until(
            &request,
            &ConservativeProbe,
            &authorizer,
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    assert!(engine.scope(&scope).unwrap().revoked);
    assert_eq!(result.stopped, Some(InspectionStop::PermissionRevoked));
    assert!(!result.confirmed());
    assert!(result.digest_hex.is_empty());
    assert_eq!(result.bytes_digested, 0);
}
