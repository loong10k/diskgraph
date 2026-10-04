//! 准备账本必须在原生捕获之前拒绝；来源：D42 EC-04，真实 Linux 索引及公开 runner。
use super::CAPTURE;
use crate::EngineError;
use crate::process_execution_fixture::{ProcessExecutionFixture, now};
use diskgraph_core::{BusinessError, ProcessEvidenceFailureCode, ProcessEvidenceLimits};
use diskgraph_store::{JobState, StoreError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 清理此测试线程的请求局部 hook，断言 panic 也不污染后续测试；来源：原生 Rust 测试同步。
struct CaptureReset;
impl Drop for CaptureReset {
    fn drop(&mut self) {
        CAPTURE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

#[test]
fn tiny_metadata_budget_rejects_preparation_before_native_capture() {
    let f = ProcessExecutionFixture::new();
    let input = f.input(
        &f.base,
        ProcessEvidenceLimits::new(15_000, 1, 32768, 65536, 8 << 20, 2, 64).unwrap(),
    );
    let job = f
        .engine
        .control_store()
        .unwrap()
        .create_process_evidence_job(&input, &f.authority(now() + 60), 8)
        .unwrap()
        .unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&reached);
    let reset = CaptureReset;
    CAPTURE.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "fixture must start with an empty request-local hook"
        );
        *slot.borrow_mut() = Some((
            job.job_id.clone(),
            Box::new(move || {
                observed.store(true, Ordering::SeqCst);
            }),
        ));
    });
    let started = Instant::now();
    let result = f
        .engine
        .run_job_strict(&job.job_id, "preparation-budget-owner");
    let consumed = reached.load(Ordering::SeqCst);
    let still_pending = CAPTURE.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|(id, _)| id == &job.job_id)
    });
    drop(reset);
    CAPTURE.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "hook cleanup must not depend on a passing assertion"
        )
    });
    assert!(
        started.elapsed() < Duration::from_millis(input.limits().max_duration_ms()),
        "deadline refusal cannot substitute for preparation raw admission"
    );
    assert!(
        matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
                | Err(EngineError::Store(StoreError::BudgetExceeded))
        ),
        "exact preparation budget result: {result:?}"
    );
    assert_eq!(
        f.engine.job_status(&job.job_id).unwrap().state,
        JobState::Failed
    );
    assert_eq!(
        f.engine
            .control_store()
            .unwrap()
            .process_job_failure(&job.job_id)
            .unwrap()
            .unwrap()
            .code(),
        ProcessEvidenceFailureCode::BudgetExceeded
    );
    f.assert_no_publication();
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    assert!(
        !consumed && still_pending,
        "preparation already owned data and reached native capture; the later native 8192-byte refusal is insufficient"
    );
}
