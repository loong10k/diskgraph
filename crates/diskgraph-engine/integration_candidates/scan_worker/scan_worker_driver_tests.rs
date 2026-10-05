//! PF-06 父驱动真实通道测试：新 API 缺失仅接口缺失，不冒称已验证运行行为。
use crate::scan_worker_driver_test_support::{DriverFixture, check};
use crate::scan_worker_failure::ScanWorkerFailure;
use diskgraph_scan_worker::ExecutionOutcome;
use std::time::Duration;

/// 不可 Clone 的原授权错误见证；来源：父请求实际 checkpoint 的错误所有权。
#[derive(Debug, PartialEq, Eq)]
struct OriginalDenial(u64);

#[test]
fn end_control_closed_and_two_eofs_do_not_deliver_while_actual_leader_is_alive() {
    let mut f = DriverFixture::new("held-end", 4096);
    let deadline = f.deadline;
    while !f.reached("pipes-closed") {
        assert!(f.driver.poll(false, || check(deadline)).unwrap().is_none());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(f.reached("request-complete") && f.reached("control-eof"));
    let mut first = None;
    let mut advanced = false;
    for _ in 0..50 {
        assert!(
            f.driver.poll(false, || check(deadline)).unwrap().is_none(),
            "End/EOF cannot substitute for actual exit"
        );
        if let Ok(bytes) = std::fs::read(f.directory.path().join("heartbeat")) {
            if let Ok(bytes) = <[u8; 8]>::try_from(bytes.as_slice()) {
                let now = u64::from_le_bytes(bytes);
                if let Some(prior) = first {
                    advanced |= now > prior;
                } else {
                    first = Some(now);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    f.release();
    let outcome = loop {
        if let Some(outcome) = f.driver.poll(false, || check(deadline)).unwrap() {
            break outcome;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        advanced,
        "real worker heartbeat must advance before release"
    );
    assert!(f.reached("natural-exit"));
    let wire: diskgraph_scan_worker::WorkerRequest = serde_json::from_slice(
        &std::fs::read(f.directory.path().join("actual-request.json")).unwrap(),
    )
    .unwrap();
    let (actual_root, actual_options, actual_limits) = wire.into_scan().unwrap();
    assert_eq!(actual_root, f.directory.path().join("tree"));
    assert_eq!(
        diskgraph_scan_worker::ScanOptions::from_native(&actual_options),
        diskgraph_scan_worker::ScanOptions::from_native(
            &crate::scan_worker_driver_test_support::options()
        )
    );
    assert_eq!(actual_limits.max_stream_bytes, 1048576);
    assert_eq!(actual_limits.max_frame_bytes, 65536);
    assert_eq!(actual_limits.max_nodes, 4096);
    assert_eq!(actual_limits.max_depth, 64);
    let ExecutionOutcome::Tree(tree) = outcome else {
        panic!("expected complete tree")
    };
    assert_eq!(tree.files, 1);
    assert_eq!(tree.children[0].name.as_ref(), "item");
    f.assert_reaped();
}

#[test]
fn pending_cancel_is_sent_only_after_complete_request_and_never_delivers_tree() {
    let mut f = DriverFixture::new("cancel-order", 4096);
    let deadline = f.deadline;
    let outcome = loop {
        if let Some(outcome) = f.driver.poll(true, || check(deadline)).unwrap() {
            break outcome;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        f.reached("request-complete")
            && f.reached("cancel-after-request")
            && f.reached("control-eof")
    );
    let ExecutionOutcome::Failure(error) = outcome else {
        panic!("Cancel cannot deliver success tree")
    };
    assert_eq!(error.code(), "cancelled");
    assert_eq!(error.io_kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(error.raw_os_error(), None);
    f.assert_reaped();
}

#[test]
fn stderr_backpressure_is_drained_while_request_and_stdout_progress() {
    let mut f = DriverFixture::new("stderr-first", 262144);
    let deadline = f.deadline;
    let outcome = loop {
        if let Some(outcome) = f.driver.poll(false, || check(deadline)).unwrap() {
            break outcome;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        f.reached("stderr-written")
            && f.reached("request-complete")
            && f.reached("control-eof")
            && f.reached("natural-exit")
    );
    assert!(matches!(outcome, ExecutionOutcome::Tree(_)));
    f.assert_reaped();
}

#[test]
fn stderr_limit_is_not_unbounded_diagnostic_memory_or_a_success_tree() {
    let mut f = DriverFixture::new("stderr-limit", 4096);
    let deadline = f.deadline;
    let error = loop {
        match f.driver.poll(false, || check(deadline)) {
            Err(error) => break error,
            Ok(None) => std::thread::sleep(Duration::from_millis(1)),
            Ok(Some(_)) => panic!("stderr limit exceeded but returned an outcome"),
        }
    };
    assert!(
        matches!(error, ScanWorkerFailure::OutputLimit { cleanup: None }),
        "{error:?}"
    );
    f.assert_reaped();
}

#[test]
fn malformed_frame_preserves_protocol_cause_and_reaps_the_real_process() {
    let mut f = DriverFixture::new("bad-frame", 4096);
    let deadline = f.deadline;
    let error = loop {
        match f.driver.poll(false, || check(deadline)) {
            Err(error) => break error,
            Ok(None) => std::thread::sleep(Duration::from_millis(1)),
            Ok(Some(_)) => panic!("bad frame returned an outcome"),
        }
    };
    assert!(f.reached("request-complete"));
    match error {
        ScanWorkerFailure::Protocol { source, cleanup } => {
            assert_eq!(source.kind(), std::io::ErrorKind::InvalidData);
            assert!(cleanup.is_none());
        }
        other => panic!("wrong primary error: {other:?}"),
    }
    f.assert_reaped();
}

#[test]
fn checkpoint_denial_retains_nonclone_primary_and_performs_error_cleanup_not_normal_exit() {
    let mut f = DriverFixture::new("checkpoint", 4096);
    let deadline = f.deadline;
    while !f.reached("checkpoint-ready") {
        assert!(f.driver.poll(false, || check(deadline)).unwrap().is_none());
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut calls = 0;
    let error = f
        .driver
        .poll(false, || {
            calls += 1;
            Err(OriginalDenial(73))
        })
        .unwrap_err();
    assert!(calls > 0);
    match error {
        ScanWorkerFailure::Checkpoint { primary, cleanup } => {
            assert_eq!(primary, OriginalDenial(73));
            assert!(cleanup.is_none());
        }
        other => panic!("primary denial was replaced: {other:?}"),
    }
    assert!(!f.reached("natural-exit"));
    f.assert_reaped();
}

#[test]
fn stdout_and_stderr_consume_one_original_response_total() {
    let mut f = DriverFixture::with_stream_limit("stderr-first", 262144, 262200);
    let deadline = f.deadline;
    let error = loop {
        match f.driver.poll(false, || check(deadline)) {
            Err(error) => break error,
            Ok(None) => std::thread::sleep(Duration::from_millis(1)),
            Ok(Some(_)) => {
                panic!("separate pipe budgets admitted more than the shared response total")
            }
        }
    };
    assert!(f.reached("stderr-written") && f.reached("request-complete"));
    assert!(
        matches!(error, ScanWorkerFailure::OutputLimit { cleanup: None }),
        "{error:?}"
    );
    f.assert_reaped();
}
