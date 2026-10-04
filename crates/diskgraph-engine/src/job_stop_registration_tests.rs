//! 真实 Map 锁等待后的停止信号必须在分发前消费；来源：原生 Rust PF-06 执行链。
//! 阶段观察点仅记录真实路径，不替换认领、取消、权限、扫描或 keeper 逻辑。

use crate::EngineError;
use crate::git_evidence_fixture::GitEvidenceFixture;
use diskgraph_core::BusinessError;
use diskgraph_store::JobState;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

type Hook = (String, Box<dyn FnOnce()>);
thread_local! {
    static REGISTRATION: RefCell<Option<Hook>> = RefCell::new(None);
    static DISPATCH: RefCell<Option<Hook>> = RefCell::new(None);
}

fn take(slot: &RefCell<Option<Hook>>, job: &str) {
    let hook = if slot.borrow().as_ref().is_some_and(|(id, _)| id == job) {
        slot.borrow_mut().take()
    } else {
        None
    };
    if let Some((_, hook)) = hook {
        hook();
    }
}

/// 参数：当前实际认领任务；返回无，只观察登记锁之前已完成的准备阶段。
pub(super) fn before_registration(job: &str) {
    REGISTRATION.with(|slot| take(slot, job));
}

/// 参数：当前实际认领任务；返回无，只观察真实 dispatch closure 是否被进入。
pub(super) fn before_dispatch(job: &str) {
    DISPATCH.with(|slot| take(slot, job));
}

#[test]
fn request_arriving_during_registration_lock_wait_stops_before_dispatch() {
    registration_wait(false);
}

#[test]
fn denial_arriving_during_registration_lock_wait_stops_before_dispatch() {
    registration_wait(true);
}

fn registration_wait(deny: bool) {
    let f = GitEvidenceFixture::new();
    let job = f
        .engine
        .index_scope(&f.scope, &f.actor, &f.engine.policy_authorizer().unwrap())
        .unwrap();
    let request = Arc::new(AtomicBool::new(false));
    let denied = Arc::new(AtomicBool::new(false));
    let dispatched = Arc::new(AtomicBool::new(false));
    std::thread::scope(|threads| {
        // 先持图锁，让真实 Index 已认领但尚未登记。不能从调用前持 Map，
        // 否则只会阻塞执行入口最初的 prior_cancel 查询，无法覆盖目标窗口。
        let graph = f.engine.graph().unwrap();
        let (reached, registration_reached) = std::sync::mpsc::channel();
        let entered = dispatched.clone();
        let execution = threads.spawn(|| {
            REGISTRATION.with(|slot| {
                *slot.borrow_mut() = Some((
                    job.job_id.clone(),
                    Box::new(move || reached.send(()).unwrap()),
                ));
            });
            DISPATCH.with(|slot| {
                *slot.borrow_mut() = Some((
                    job.job_id.clone(),
                    Box::new(move || entered.store(true, Ordering::SeqCst)),
                ));
            });
            let result = f.engine.run_job_with_stop_signals(
                &job.job_id,
                "registration-wait",
                request.clone(),
                denied.clone(),
            );
            REGISTRATION.with(|slot| slot.borrow_mut().take());
            DISPATCH.with(|slot| slot.borrow_mut().take());
            result
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let running = loop {
            let record = f.engine.job_status(&job.job_id).unwrap();
            if record.state == JobState::Running {
                break record;
            }
            assert!(Instant::now() < deadline, "actual claim never completed");
            std::thread::sleep(Duration::from_millis(2));
        };
        let held_map = f.engine.cancellations().unwrap();
        assert!(!held_map.contains_key(&job.job_id));
        drop(graph);
        // observer 位于第二次 bridge.check 之后；真实 Map guard 始终未释放。
        // 信号在该 observer 之后才设置，不让原有图锁末检覆盖新增登记等待窗口。
        let reached = registration_reached.recv_timeout(Duration::from_secs(10));
        let before_signal = f.engine.job_status(&job.job_id).unwrap();
        let prior_cancel = f
            .engine
            .control_store()
            .unwrap()
            .cancellation_requested(&job.job_id, running.fencing_token)
            .unwrap();
        denied.store(deny, Ordering::SeqCst);
        request.store(true, Ordering::SeqCst);
        drop(held_map);
        let result = execution.join().unwrap();
        // 先释放真实锁并 join 后断言，失败不会把被测执行器永久留在测试锁上。
        assert!(
            reached.is_ok(),
            "registration phase not reached: {reached:?}"
        );
        assert_eq!(before_signal, running);
        assert!(
            !prior_cancel,
            "signal was already persisted before the lock window"
        );
        assert!(
            !dispatched.load(Ordering::SeqCst),
            "pending stop crossed registration Mutex into actual dispatch: {result:?}"
        );
        if deny {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Business(BusinessError::PermissionDenied))
                ),
                "verified denial lost its original result: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
                "same-generation caller request was not consumed: {result:?}"
            );
        }
        let terminal = f.engine.job_status(&job.job_id).unwrap();
        assert_eq!(
            terminal.state,
            if deny {
                JobState::Failed
            } else {
                JobState::Cancelled
            }
        );
        assert_eq!(terminal.owner, running.owner);
        assert_eq!(terminal.fencing_token, running.fencing_token);
        assert_eq!(
            f.engine
                .control_store()
                .unwrap()
                .cancellation_requested(&job.job_id, running.fencing_token)
                .unwrap(),
            !deny
        );
    });
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    f.assert_no_git_publication();
}
