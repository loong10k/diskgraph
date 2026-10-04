//! 执行 panic 的真实续租线程退出回归；来源：有效 Git 任务及发布前同步点。
use crate::git_evidence_fixture::{GitEvidenceFixture, now};
use std::time::Duration;

#[test]
fn execution_unwind_joins_keeper_without_revocation_or_rescue_cancellation() {
    let f = GitEvidenceFixture::new();
    let job = f.enqueue(&f.base, now() + 300);
    let engine = f.engine.clone();
    let job_id = job.job_id.clone();
    let (reached, qualified) = std::sync::mpsc::channel();
    let (done, finished) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let observed_engine = engine.clone();
        let observed_job = job_id.clone();
        crate::git_evidence_execution_tests::at_publication(&job_id, move || {
            let mut control = observed_engine.control_store().unwrap();
            let running = control.job(&observed_job).unwrap();
            assert_eq!(running.state, diskgraph_store::JobState::Running);
            assert_eq!(running.owner, "unwind-owner");
            assert!(
                u128::from(running.lease_expires_unix_ms)
                    > std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis()
            );
            control
                .with_job_fence(&observed_job, "unwind-owner", running.fencing_token, || {
                    Ok(())
                })
                .unwrap();
            drop(control);
            reached.send(running.fencing_token).unwrap();
            std::panic::panic_any("qualified execution panic");
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.run_job_strict(&job_id, "unwind-owner")
        }));
        done.send(result).unwrap();
    });
    let phase = qualified.recv_timeout(Duration::from_secs(20));
    let completed = if phase.is_ok() {
        finished.recv_timeout(Duration::from_secs(2))
    } else {
        finished
            .try_recv()
            .map_err(|_| std::sync::mpsc::RecvTimeoutError::Timeout)
    };
    // RED 也有明确清理路径，避免回归自身遗留续租线程；此救援不能计为通过。
    let rescued = completed.is_err();
    if rescued {
        f.engine
            .cancel_job(
                &job.job_id,
                &f.actor,
                &f.engine.policy_authorizer().unwrap(),
            )
            .unwrap();
    }
    let result = match completed {
        Ok(result) => result,
        Err(_) => finished
            .recv_timeout(Duration::from_secs(5))
            .expect("rescue must release keeper"),
    };
    worker.join().unwrap();
    assert!(
        phase.is_ok(),
        "must reach an authorized live owner before panic: {phase:?}"
    );
    assert!(
        !rescued,
        "execution panic was stuck joining keeper until extra cancellation"
    );
    let panic = result.expect_err("original panic must propagate");
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"qualified execution panic")
    );
    assert!(!f.engine.cancellations().unwrap().contains_key(&job.job_id));
    assert!(
        !f.engine
            .scan_progress
            .lock()
            .unwrap()
            .keys()
            .any(|(id, _)| id == &job.job_id)
    );
    f.assert_no_git_publication();
}
