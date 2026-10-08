//! 内容初始准入必须沿原期限等待控制锁；来源：DiskGraph CT-01/02 原生契约。
use super::read_terminal_fixture::ReadTerminalFixture;
use super::{ConservativeProbe, InspectionRequest};
use crate::EngineError;
use diskgraph_core::BusinessError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[test]
fn content_initial_authorization_returns_before_original_control_owner_releases() {
    for digest in [false, true] {
        let fixture = ReadTerminalFixture::new(true);
        let released = AtomicBool::new(false);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        std::thread::scope(|threads| {
            let engine = &fixture.engine;
            let owner_released = &released;
            let owner = threads.spawn(move || {
                let control = engine.control_store().unwrap();
                ready_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_millis(500));
                drop(control);
                owner_released.store(true, Ordering::SeqCst);
            });
            ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let request = InspectionRequest {
                scope_id: &fixture.scope,
                principal: &fixture.principal,
                path: &fixture.path,
                offset: 0,
                max_bytes: 8,
                cancel: None,
                chunk_bytes: 8,
            };
            let deadline = Instant::now() + Duration::from_millis(50);
            let result = if digest {
                fixture
                    .engine
                    .digest_bounded_until(&request, &ConservativeProbe, &fixture.policy, deadline)
                    .map(|_| ())
            } else {
                fixture
                    .engine
                    .read_bounded_until(&request, &ConservativeProbe, &fixture.policy, deadline)
                    .map(|_| ())
            };
            let returned_while_held = !released.load(Ordering::SeqCst);
            let _ = release_tx.send(());
            owner.join().unwrap();
            assert!(
                returned_while_held,
                "content waited past original deadline; digest={digest}"
            );
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::BudgetExceeded))
                ),
                "digest={digest}: {result:?}"
            );
        });
    }
}

#[test]
fn body_refuses_result_when_chunk_authorization_exhausts_original_deadline() {
    struct SlowChunk<'a> {
        policy: &'a diskgraph_core::PolicyAuthorizer,
        calls: std::cell::Cell<usize>,
    }
    impl diskgraph_core::Authorizer for SlowChunk<'_> {
        fn decide(
            &self,
            principal: &diskgraph_core::PrincipalId,
            permission: &diskgraph_core::Permission,
            scope: &diskgraph_core::ScopeId,
        ) -> diskgraph_core::Decision {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == 2 {
                std::thread::sleep(Duration::from_millis(80));
            }
            diskgraph_core::Authorizer::decide(self.policy, principal, permission, scope)
        }
    }
    let fixture = ReadTerminalFixture::new(true);
    let authorizer = SlowChunk {
        policy: &fixture.policy,
        calls: std::cell::Cell::new(0),
    };
    let request = InspectionRequest {
        scope_id: &fixture.scope,
        principal: &fixture.principal,
        path: &fixture.path,
        offset: 0,
        max_bytes: 8,
        cancel: None,
        chunk_bytes: 1,
    };
    let result = fixture.engine.read_bounded_until(
        &request,
        &ConservativeProbe,
        &authorizer,
        Instant::now() + Duration::from_millis(50),
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ),
        "{result:?}"
    );
}
