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
