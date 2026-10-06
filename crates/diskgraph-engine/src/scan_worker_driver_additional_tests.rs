//! 父驱动补充回归；原七个冻结 Child 测试与夹具不变，不把 DTO 单测当真实扫描资格。
use crate::scan_worker_driver_test_support::{DriverFixture, PIN, TARGET, check};
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_output::ScanWorkerOutput;
use diskgraph_scan_worker::{ExecutionFrame, FrameWriter, ProtocolLimits, ScanProgress};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

#[test]
fn actual_progress_dto_is_retained_without_fabricated_node_counters() {
    let limits = ProtocolLimits {
        max_frame_bytes: 4096,
        max_stream_bytes: 8192,
        max_nodes: 10,
        max_depth: 4,
    };
    let expected = ScanProgress {
        files: 17,
        dirs: 3,
        bytes: 912,
        errors: 2,
        finished: true,
        cancelled: false,
        messages: vec!["bounded upstream message".into(), "第二条实际字段".into()],
    };
    let mut bytes = Vec::new();
    {
        let mut writer = FrameWriter::new(&mut bytes, limits);
        writer
            .write_payload(&ExecutionFrame::<String, String>::Hello {
                version: 2,
                target: TARGET.into(),
                pin: PIN.into(),
            })
            .unwrap();
        writer
            .write_payload(&ExecutionFrame::<String, String>::Progress {
                progress: expected.clone(),
            })
            .unwrap();
    }
    let mut output = ScanWorkerOutput::new(limits, (TARGET, PIN), 4096).unwrap();
    let mut calls = 0;
    for fragment in bytes.chunks(3) {
        output
            .stdout(fragment, &mut || {
                calls += 1;
                Ok::<(), ScanWorkerFailure<std::convert::Infallible>>(())
            })
            .unwrap();
    }
    assert_eq!(output.progress(), Some(&expected));
    assert!(calls > 0);
    assert!(!output.terminal(), "finished progress is not execution End");
}

#[test]
fn caller_cancel_after_tree_eof_still_rejects_tree_and_cleans_actual_child() {
    let mut fixture = DriverFixture::new("held-end", 4096);
    let deadline = fixture.deadline;
    while !fixture.reached("pipes-closed") {
        assert!(
            fixture
                .driver
                .poll(false, || check(deadline))
                .unwrap()
                .is_none()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(fixture.reached("request-complete") && fixture.reached("control-eof"));
    let error = fixture.driver.poll(true, || check(deadline)).unwrap_err();
    assert!(
        matches!(error, ScanWorkerFailure::Cancelled { cleanup: None }),
        "{error:?}"
    );
    assert!(!fixture.reached("natural-exit"));
    fixture.assert_reaped();
}

#[test]
fn checkpoint_panic_keeps_payload_and_cleans_even_when_caller_retains_driver() {
    let mut fixture = DriverFixture::new("checkpoint", 4096);
    let deadline = fixture.deadline;
    while !fixture.reached("checkpoint-ready") {
        assert!(
            fixture
                .driver
                .poll(false, || check(deadline))
                .unwrap()
                .is_none()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let payload = catch_unwind(AssertUnwindSafe(|| {
        let _ = fixture.driver.poll(false, || -> Result<(), ()> {
            panic!("original-driver-checkpoint-panic")
        });
    }))
    .unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"original-driver-checkpoint-panic")
    );
    assert!(!fixture.reached("natural-exit"));
    fixture.assert_reaped();
    assert!(fixture.driver.unwind_cleanup_error().is_none());
    assert!(matches!(
        fixture.driver.poll(false, || check(deadline)),
        Err(ScanWorkerFailure::Stopped { cleanup: None })
    ));
}
