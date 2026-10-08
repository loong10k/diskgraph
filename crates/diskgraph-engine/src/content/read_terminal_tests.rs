//! D24 库内实际读取、身份终检之后的确定性同步；不进入生产请求状态。

use super::read_terminal_fixture::ReadTerminalFixture;
use super::{ConservativeProbe, InspectionRequest, InspectionStop};
use crate::EngineError;
use diskgraph_core::{Authorizer, BusinessError, Decision, Permission};
use diskgraph_store::ControlStore;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static BEFORE_REPLY: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(super) fn before_reply() {
    if let Some(callback) = BEFORE_REPLY.with(|slot| slot.borrow_mut().take()) {
        callback();
    }
}

fn read(
    fixture: &ReadTerminalFixture,
    max_bytes: u64,
    cancel: Option<&AtomicBool>,
) -> Result<super::ReadOutcome, EngineError> {
    fixture.engine.read_bounded(
        &InspectionRequest {
            scope_id: &fixture.scope,
            principal: &fixture.principal,
            path: &fixture.path,
            offset: 0,
            max_bytes,
            cancel,
            chunk_bytes: 8,
        },
        &ConservativeProbe,
        &fixture.policy,
    )
}

fn cancellation_case(max_bytes: u64) {
    let fixture = ReadTerminalFixture::new(true);
    let cancel = Arc::new(AtomicBool::new(false));
    let callback_cancel = cancel.clone();
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            callback_cancel.store(true, Ordering::SeqCst)
        }));
    });
    let result = read(&fixture, max_bytes, Some(&cancel)).unwrap();
    assert!(
        BEFORE_REPLY.with(|slot| slot.borrow().is_none()),
        "terminal seam was not reached"
    );
    assert_eq!(result.bytes, b"12345678");
    assert_eq!(
        result.stopped,
        Some(InspectionStop::Cancelled),
        "late cancellation was lost"
    );
}

#[test]
fn completed_range_rechecks_cancellation_after_identity_validation() {
    cancellation_case(8);
}

#[test]
fn eof_rechecks_cancellation_after_identity_validation() {
    cancellation_case(9);
}

fn scope_revocation_case(max_bytes: u64, persistent_policy: bool) {
    let fixture = ReadTerminalFixture::new(persistent_policy);
    let path = fixture
        .directory
        .path()
        .join("data/diskgraph-control.sqlite");
    let scope = fixture.scope.clone();
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            ControlStore::open(&path)
                .unwrap()
                .revoke_scope(&scope)
                .unwrap();
        }));
    });
    let result = read(&fixture, max_bytes, None);
    assert!(
        BEFORE_REPLY.with(|slot| slot.borrow().is_none()),
        "terminal seam was not reached"
    );
    assert!(fixture.engine.scope(&fixture.scope).unwrap().revoked);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "returned body after terminal scope revocation"
    );
}

#[test]
fn completed_range_refuses_actual_scope_revoked_at_return() {
    scope_revocation_case(8, true);
}

#[test]
fn eof_refuses_actual_scope_revoked_at_return_without_persistent_policy() {
    scope_revocation_case(9, false);
}

fn grant_revocation_case(max_bytes: u64) {
    let fixture = ReadTerminalFixture::new(true);
    let path = fixture
        .directory
        .path()
        .join("data/diskgraph-control.sqlite");
    let scope = fixture.scope.clone();
    let principal = fixture.principal.clone();
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            ControlStore::open(&path)
                .unwrap()
                .revoke_grant(&principal, &Permission::ContentRead, &scope)
                .unwrap();
        }));
    });
    let result = read(&fixture, max_bytes, None);
    assert!(
        BEFORE_REPLY.with(|slot| slot.borrow().is_none()),
        "terminal seam was not reached"
    );
    assert!(
        matches!(
            fixture.engine.policy_authorizer().unwrap().decide(
                &fixture.principal,
                &Permission::ContentRead,
                &fixture.other
            ),
            Decision::Allowed
        ),
        "other scope must retain its grant"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "returned body after single-grant revocation"
    );
}

#[test]
fn completed_range_refuses_single_content_grant_revoked_at_return() {
    grant_revocation_case(8);
}

#[test]
fn eof_refuses_single_content_grant_revoked_at_return() {
    grant_revocation_case(9);
}

#[test]
fn content_terminal_capability_callback_can_read_control_without_reentrant_lock() {
    struct ReadsControl<'a> {
        engine: &'a crate::Engine,
        policy: diskgraph_core::PolicyAuthorizer,
        observed: std::cell::Cell<bool>,
    }
    impl Authorizer for ReadsControl<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> Decision {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
            let read = self
                .engine
                .control_until(deadline)
                .and_then(|control| control.existing_server_id().map_err(EngineError::from));
            self.observed.set(read.is_ok());
            self.policy.decide(principal, permission, scope)
        }
    }
    // 仅验证终态授权锁边界；不绕过原生文件内容能力门禁。
    let (dir, engine, principal, scope, _) =
        crate::relation_request_tests::published_authorization_fixture();
    engine.set_content_read(&scope, &principal, true).unwrap();
    let authorizer = ReadsControl {
        engine: &engine,
        policy: engine.policy_authorizer().unwrap(),
        observed: std::cell::Cell::new(false),
    };
    let path = dir.path().join("permission-only");
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 1,
        cancel: None,
        chunk_bytes: 1,
    };
    engine.require_read_terminal(&request, &authorizer).unwrap();
    assert!(
        authorizer.observed.get(),
        "content terminal callback held the control lock"
    );
}

#[test]
fn permission_only_terminal_refuses_callback_scope_or_grant_revocation() {
    struct Revokes {
        policy: diskgraph_core::PolicyAuthorizer,
        control: RefCell<ControlStore>,
        revoke_scope: bool,
    }
    impl Authorizer for Revokes {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> Decision {
            if self.revoke_scope {
                self.control.borrow_mut().revoke_scope(scope).unwrap();
            } else {
                self.control
                    .borrow_mut()
                    .revoke_grant(principal, permission, scope)
                    .unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for mode in 0..3 {
        let (dir, engine, principal, scope, _) =
            crate::relation_request_tests::published_authorization_fixture();
        engine.set_content_read(&scope, &principal, true).unwrap();
        let policy = engine.policy_authorizer().unwrap();
        let db = dir.path().join("data/diskgraph-control.sqlite");
        if mode == 2 {
            rusqlite::Connection::open(&db)
                .unwrap()
                .execute_batch("DELETE FROM grants; DELETE FROM policy;")
                .unwrap();
        }
        let authorizer = Revokes {
            policy,
            control: RefCell::new(ControlStore::open(&db).unwrap()),
            revoke_scope: mode != 0,
        };
        let path = dir.path().join("permission-only");
        let request = InspectionRequest {
            scope_id: &scope,
            principal: &principal,
            path: &path,
            offset: 0,
            max_bytes: 1,
            cancel: None,
            chunk_bytes: 1,
        };
        assert!(
            matches!(
                engine.require_read_terminal(&request, &authorizer),
                Err(EngineError::Business(BusinessError::PermissionDenied))
            ),
            "callback revocation mode {mode}"
        );
    }
}

#[test]
fn permission_only_content_terminal_refuses_late_allow_after_fixed_expiry() {
    struct Expires {
        policy: diskgraph_core::PolicyAuthorizer,
        expiry: u64,
        entered_live: std::cell::Cell<bool>,
    }
    impl Authorizer for Expires {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> Decision {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap();
            self.entered_live.set(now.as_secs() < self.expiry);
            let decision = self.policy.decide(principal, permission, scope);
            std::thread::sleep(
                std::time::Duration::from_secs(self.expiry).saturating_sub(now)
                    + std::time::Duration::from_millis(30),
            );
            decision
        }
        fn expires_at_unix_seconds(&self) -> Option<u64> {
            Some(self.expiry)
        }
    }
    // 权限终态专用回归：不打开资源文件，不替代实际 read_bounded 平台验收。
    let (dir, engine, principal, scope, _) =
        crate::relation_request_tests::published_authorization_fixture();
    engine.set_content_read(&scope, &principal, true).unwrap();
    let authorizer = Expires {
        policy: engine.policy_authorizer().unwrap(),
        expiry: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 2,
        entered_live: std::cell::Cell::new(false),
    };
    let path = dir.path().join("permission-only");
    let request = InspectionRequest {
        scope_id: &scope,
        principal: &principal,
        path: &path,
        offset: 0,
        max_bytes: 1,
        cancel: None,
        chunk_bytes: 1,
    };
    let result = engine.require_read_terminal(&request, &authorizer);
    assert!(
        authorizer.entered_live.get(),
        "fixture must enter capability before fixed expiry"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::PermissionDenied))
        ),
        "expired content terminal returned: {result:?}"
    );
}

#[test]
fn placeholder_digest_preserves_revocation_without_confirming_content() {
    /// 在占位诊断期间撤销原范围；仅同步竞态，不替代原生访问保护。
    struct RevokingProbe {
        control_path: std::path::PathBuf,
        scope: diskgraph_core::ScopeId,
    }
    impl super::PlaceholderProbe for RevokingProbe {
        fn is_placeholder(&self, _: &std::path::Path) -> bool {
            ControlStore::open(&self.control_path)
                .unwrap()
                .revoke_scope(&self.scope)
                .unwrap();
            true
        }
    }
    for persistent_policy in [false, true] {
        let fixture = ReadTerminalFixture::new(persistent_policy);
        let probe = RevokingProbe {
            control_path: fixture
                .directory
                .path()
                .join("data/diskgraph-control.sqlite"),
            scope: fixture.scope.clone(),
        };
        let outcome = fixture
            .engine
            .digest_bounded_until(
                &InspectionRequest {
                    scope_id: &fixture.scope,
                    principal: &fixture.principal,
                    path: &fixture.path,
                    offset: 0,
                    max_bytes: 8,
                    cancel: None,
                    chunk_bytes: 8,
                },
                &probe,
                &fixture.policy,
                std::time::Instant::now() + std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(outcome.stopped, Some(InspectionStop::PermissionRevoked));
        assert_eq!(outcome.bytes_digested, 0);
        assert!(outcome.digest_hex.is_empty());
    }
}

#[test]
fn placeholder_digest_rechecks_cancel_and_deadline_after_probe() {
    /// 在真实占位诊断调用中触发取消或跨越原期限。
    struct StoppingProbe<'a> {
        cancel: &'a AtomicBool,
        deadline: std::time::Instant,
        expire: bool,
        entered: std::cell::Cell<bool>,
    }
    impl super::PlaceholderProbe for StoppingProbe<'_> {
        fn is_placeholder(&self, _: &std::path::Path) -> bool {
            self.entered.set(true);
            if self.expire {
                std::thread::sleep(
                    self.deadline
                        .saturating_duration_since(std::time::Instant::now())
                        + std::time::Duration::from_millis(1),
                );
            } else {
                self.cancel.store(true, Ordering::SeqCst);
            }
            true
        }
    }
    for expire in [false, true] {
        let fixture = ReadTerminalFixture::new(true);
        let cancel = AtomicBool::new(false);
        let deadline = std::time::Instant::now()
            + if expire {
                std::time::Duration::from_millis(20)
            } else {
                std::time::Duration::from_secs(30)
            };
        let probe = StoppingProbe {
            cancel: &cancel,
            deadline,
            expire,
            entered: std::cell::Cell::new(false),
        };
        let outcome = fixture
            .engine
            .digest_bounded_until(
                &InspectionRequest {
                    scope_id: &fixture.scope,
                    principal: &fixture.principal,
                    path: &fixture.path,
                    offset: 0,
                    max_bytes: 8,
                    cancel: Some(&cancel),
                    chunk_bytes: 8,
                },
                &probe,
                &fixture.policy,
                deadline,
            )
            .unwrap();
        assert!(probe.entered.get());
        assert_eq!(
            outcome.stopped,
            Some(if expire {
                InspectionStop::Deadline
            } else {
                InspectionStop::Cancelled
            })
        );
        assert_eq!(outcome.bytes_digested, 0);
        assert!(outcome.digest_hex.is_empty());
    }
}

#[test]
fn body_does_not_escape_when_terminal_authorization_misses_original_deadline() {
    let fixture = ReadTerminalFixture::new(true);
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(|| {
            std::thread::sleep(std::time::Duration::from_millis(80));
        }));
    });
    let request = InspectionRequest {
        scope_id: &fixture.scope,
        principal: &fixture.principal,
        path: &fixture.path,
        offset: 0,
        max_bytes: 8,
        cancel: None,
        chunk_bytes: 8,
    };
    let result = fixture.engine.read_bounded_until(
        &request,
        &ConservativeProbe,
        &fixture.policy,
        std::time::Instant::now() + std::time::Duration::from_millis(50),
    );
    assert!(BEFORE_REPLY.with(|slot| slot.borrow().is_none()));
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "{result:?}"
    );
}

#[test]
fn body_refuses_content_grant_withdrawn_and_restored_before_return() {
    let fixture = ReadTerminalFixture::new(true);
    let path = fixture
        .directory
        .path()
        .join("data/diskgraph-control.sqlite");
    let principal = fixture.principal.clone();
    let scope = fixture.scope.clone();
    BEFORE_REPLY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            let mut control = ControlStore::open(&path).unwrap();
            let policy_version = control.policy_version().unwrap();
            control
                .revoke_grant(&principal, &Permission::ContentRead, &scope)
                .unwrap();
            control
                .upsert_grant(&diskgraph_core::Grant {
                    principal,
                    permission: Permission::ContentRead,
                    scope,
                    policy_version,
                })
                .unwrap();
        }));
    });
    let result = read(&fixture, 8, None);
    assert!(
        matches!(
            result,
            Err(EngineError::Business(
                BusinessError::PermissionDenied | BusinessError::Conflict
            ))
        ),
        "{result:?}"
    );
}

#[test]
fn digest_never_confirms_restored_grant_at_initial_chunk_or_terminal_callback() {
    struct Churn<'a> {
        policy: &'a diskgraph_core::PolicyAuthorizer,
        control: RefCell<ControlStore>,
        calls: std::cell::Cell<usize>,
        churn_at: usize,
    }
    impl Authorizer for Churn<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> Decision {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == self.churn_at {
                let mut control = self.control.borrow_mut();
                let policy_version = control.policy_version().unwrap();
                control.revoke_grant(principal, permission, scope).unwrap();
                control
                    .upsert_grant(&diskgraph_core::Grant {
                        principal: principal.clone(),
                        permission: permission.clone(),
                        scope: scope.clone(),
                        policy_version,
                    })
                    .unwrap();
            }
            self.policy.decide(principal, permission, scope)
        }
    }
    for churn_at in 1..=4 {
        let fixture = ReadTerminalFixture::new(true);
        let authorizer = Churn {
            policy: &fixture.policy,
            control: RefCell::new(
                ControlStore::open(
                    &fixture
                        .directory
                        .path()
                        .join("data/diskgraph-control.sqlite"),
                )
                .unwrap(),
            ),
            calls: std::cell::Cell::new(0),
            churn_at,
        };
        let request = InspectionRequest {
            scope_id: &fixture.scope,
            principal: &fixture.principal,
            path: &fixture.path,
            offset: 0,
            max_bytes: 8,
            cancel: None,
            chunk_bytes: 8,
        };
        let result = fixture.engine.digest_bounded_until(
            &request,
            &ConservativeProbe,
            &authorizer,
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        );
        assert_eq!(
            authorizer.calls.get(),
            churn_at,
            "callback stage not reached"
        );
        if churn_at == 1 {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(
                        BusinessError::PermissionDenied | BusinessError::Conflict
                    ))
                ),
                "{result:?}"
            );
        } else {
            let outcome = result.unwrap();
            assert!(
                outcome.digest_hex.is_empty(),
                "confirmed restored grant at {churn_at}"
            );
            assert!(matches!(
                outcome.stopped,
                Some(InspectionStop::PermissionRevoked | InspectionStop::ReadError)
            ));
            assert_eq!(outcome.bytes_digested, if churn_at >= 3 { 8 } else { 0 });
        }
    }
}
