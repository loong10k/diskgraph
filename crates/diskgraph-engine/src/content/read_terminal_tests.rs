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
