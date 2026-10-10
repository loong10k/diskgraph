//! 首次实时授权释放锁后的确定性撤权竞态回归；来源：DiskGraph 原生安全契约。
use crate::Engine;
use std::cell::RefCell;

type AfterAuthorization = Box<dyn FnOnce(&Engine)>;
thread_local! {
    static OBSERVER: RefCell<Option<AfterAuthorization>> = const { RefCell::new(None) };
}

/// 参数：engine 是刚完成首次授权的原引擎；返回：无，先释放 TLS 借用再执行观察。
pub(super) fn after_initial_authorization(engine: &Engine) {
    let observer = OBSERVER.with(|slot| slot.borrow_mut().take());
    if let Some(observer) = observer {
        observer(engine);
    }
}

#[test]
fn continuous_revocation_between_initial_locks_has_denial_priority() {
    let (_dir, engine, principal, scope, revision) =
        crate::relation_request_tests::published_authorization_fixture();
    let policy = engine.policy_authorizer().unwrap();
    let actor = principal.clone();
    OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |engine| {
            // 原生产授权已完成；在后续取锁前实际提交撤权，不用 sleep 猜测竞态。
            engine
                .control_store()
                .unwrap()
                .revoke_grant(&actor, &diskgraph_core::Permission::MetadataRead, &scope)
                .unwrap();
        }))
    });
    let entered = std::cell::Cell::new(false);
    let result = engine.with_authorized_revision_reader_until(
        revision,
        &principal,
        &policy,
        std::time::Instant::now() + std::time::Duration::from_secs(1),
        |_, _, _| {
            entered.set(true);
            Ok(())
        },
    );
    assert!(
        matches!(
            result,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::PermissionDenied
            ))
        ),
        "{result:?}"
    );
    assert!(!entered.get(), "revoked request entered consumer");
}

#[test]
fn terminal_reader_waits_for_brief_contention_but_keeps_its_observation_window() {
    use diskgraph_core::{Authorizer, Decision, Permission, PrincipalId, ScopeId};
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    // 第二次能力回调精确对应终检；先确认另一线程持有真实控制锁再返回。
    /// 在终检回调同步制造真实控制锁竞争，验证短竞争与超时的不同结果。
    struct Contended<F> {
        calls: Cell<u32>,
        terminal_started: Cell<Option<Instant>>,
        callback_elapsed: Cell<Duration>,
        contend: F,
    }
    impl<F: Fn()> Authorizer for Contended<F> {
        fn decide(&self, _: &PrincipalId, _: &Permission, _: &ScopeId) -> Decision {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == 2 {
                let callback_started = Instant::now();
                (self.contend)();
                self.callback_elapsed.set(callback_started.elapsed());
                self.terminal_started.set(Some(Instant::now()));
            }
            Decision::Allowed
        }
    }
    for (hold_ms, request_ms, allowed) in [
        (0, 1000, true),
        (100, 1000, true),
        (500, 1000, false),
        (100, 60, false),
    ] {
        let (_dir, engine, principal, _, revision) =
            crate::relation_request_tests::published_authorization_fixture();
        // 短竞争仅在原取锁循环实际观察 WouldBlock 后释放，不依赖 sleep 准时唤醒。
        let held_elapsed = std::sync::Mutex::new(None);
        let observed_wait = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|threads| {
            let policy = Contended {
                calls: Cell::new(0),
                terminal_started: Cell::new(None),
                callback_elapsed: Cell::new(Duration::ZERO),
                contend: || {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (wait_tx, wait_rx) = std::sync::mpsc::channel();
                    crate::scan_observation_deadline_tests::observe_next_control_wait(wait_tx);
                    let observed_wait = &observed_wait;
                    let engine = &engine;
                    let held_elapsed = &held_elapsed;
                    threads.spawn(move || {
                        let held = engine.control_store().unwrap();
                        let held_started = Instant::now();
                        tx.send(()).unwrap();
                        let actually_waiting = wait_rx.recv_timeout(Duration::from_secs(2)).is_ok();
                        observed_wait.store(actually_waiting, std::sync::atomic::Ordering::SeqCst);
                        if hold_ms > 0 {
                            std::thread::sleep(Duration::from_millis(hold_ms));
                        }
                        drop(held);
                        *held_elapsed.lock().unwrap() = Some(held_started.elapsed());
                    });
                    rx.recv_timeout(Duration::from_secs(2)).unwrap();
                },
            };
            let started = Instant::now();
            let result = engine.with_authorized_revision_reader_until(
                revision,
                &principal,
                &policy,
                started + Duration::from_millis(request_ms),
                |_, _, _| Ok(()),
            );
            let elapsed = policy.terminal_started.get().unwrap().elapsed();
            let timing = format!(
                "hold_ms={hold_ms}, request_ms={request_ms}, allowed={allowed}, actual_hold={:?}, callback={:?}, terminal={elapsed:?}, request={:?}",
                *held_elapsed.lock().unwrap(),
                policy.callback_elapsed.get(),
                started.elapsed(),
            );
            eprintln!("terminal_contention: {timing}; result={result:?}");
            assert_eq!(policy.calls.get(), 2);
            assert!(
                observed_wait.load(std::sync::atomic::Ordering::SeqCst),
                "must witness actual control lock contention"
            );
            if allowed {
                assert!(
                    result.is_ok(),
                    "brief terminal contention: {result:?}; {timing}"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(crate::EngineError::Business(
                            diskgraph_core::BusinessError::BudgetExceeded
                        ))
                    ),
                    "held terminal contention: {result:?}; {timing}"
                );
                assert!(elapsed < Duration::from_millis(300), "{elapsed:?}");
            }
        });
    }
}

#[test]
fn known_terminal_withdrawal_wins_after_original_reader_deadline() {
    use diskgraph_core::{Authorizer, Decision, Permission, PrincipalId, ScopeId};
    use std::cell::{Cell, RefCell};
    use std::time::{Duration, Instant};
    /// 在终检实际提交撤权后耗尽原期限，能力仍返回缓存允许。
    struct Withdrawn {
        calls: Cell<u32>,
        control: RefCell<diskgraph_store::ControlStore>,
        deadline: Instant,
    }
    impl Authorizer for Withdrawn {
        fn decide(&self, _: &PrincipalId, _: &Permission, scope: &ScopeId) -> Decision {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() == 2 {
                self.control.borrow_mut().revoke_scope(scope).unwrap();
                std::thread::sleep(
                    self.deadline.saturating_duration_since(Instant::now())
                        + Duration::from_millis(5),
                );
            }
            Decision::Allowed
        }
    }
    let (dir, engine, principal, _scope, revision) =
        crate::relation_request_tests::published_authorization_fixture();
    let control =
        diskgraph_store::ControlStore::open(&dir.path().join("data/diskgraph-control.sqlite"))
            .unwrap();
    let deadline = Instant::now() + Duration::from_millis(200);
    let policy = Withdrawn {
        calls: Cell::new(0),
        control: RefCell::new(control),
        deadline,
    };
    let result = engine.with_authorized_revision_reader_until(
        revision,
        &principal,
        &policy,
        deadline,
        |_, _, _| Ok(()),
    );
    assert_eq!(policy.calls.get(), 2);
    assert!(
        matches!(
            result,
            Err(crate::EngineError::Business(
                diskgraph_core::BusinessError::PermissionDenied
            ))
        ),
        "withdrawal masked: {result:?}"
    );
}
