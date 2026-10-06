//! PF-06 原失败和处置 owner 的实际 macOS 边界；来源：真实 Child、EOF、waitid/ECHILD。
#![cfg(target_os = "macos")]

use crate::native_child::ChildError;
use crate::scan_worker_driver::ScanWorkerDriver;
use crate::scan_worker_driver_test_support::{PIN, TARGET};
use crate::scan_worker_failure::ScanWorkerFailure;
use crate::scan_worker_owned_failure::ScanWorkerOwnedFailure;
use crate::scan_worker_owned_failure_test_support::OwnedFailureFixture;
use diskgraph_scan_worker::WorkerRequest;
use std::convert::Infallible;
use std::io;

#[test]
fn invalid_constructor_preserves_protocol_error_and_returns_no_owner_after_actual_cleanup() {
    let mut fixture = OwnedFailureFixture::new();
    let mut child = Some(fixture.take_child());
    let result = ScanWorkerDriver::new(
        &mut child,
        WorkerRequest::Cancel {},
        (TARGET, PIN),
        fixture.deadline,
        4096,
    );
    let failure: ScanWorkerOwnedFailure<Infallible> = match result {
        Err(failure) => ScanWorkerOwnedFailure::dispose(
            failure,
            child
                .take()
                .expect("rejected configuration retained original owner"),
        ),
        Ok(driver) => {
            drop(driver);
            panic!("Cancel cannot initialize a driver");
        }
    };
    let (failure, owner) = failure.into_parts();
    // 所有真实 child 的处置先发生，再断言诊断，防止断言失败成为残留进程原因。
    if let Some(mut unexpected) = owner {
        let _ = unexpected.cleanup();
        panic!("cleanup succeeded but retained owner was returned");
    }
    fixture.assert_reaped();
    match failure {
        ScanWorkerFailure::Protocol { source, cleanup } => {
            assert_eq!(source.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                source.to_string(),
                "expected one execution Request version 2"
            );
            assert!(cleanup.is_none());
        }
        other => panic!("constructor changed the original classification: {other:?}"),
    }
}

#[test]
fn actual_external_reap_constructor_failure_returns_the_original_unresolved_owner() {
    let mut fixture = OwnedFailureFixture::new();
    let mut unrelated = OwnedFailureFixture::new();
    fixture.external_reap();
    let mut child = Some(fixture.take_child());
    let result = ScanWorkerDriver::new(
        &mut child,
        WorkerRequest::Cancel {},
        (TARGET, PIN),
        fixture.deadline,
        4096,
    );
    let failure: ScanWorkerOwnedFailure<Infallible> = match result {
        Err(failure) => ScanWorkerOwnedFailure::dispose(
            failure,
            child
                .take()
                .expect("rejected configuration retained original owner"),
        ),
        Ok(driver) => {
            drop(driver);
            panic!("invalid Request was accepted");
        }
    };
    let (failure, owner) = failure.into_parts();
    let transferred = owner.is_some();
    let disposition = owner.map(|mut child| {
        // 实際 wait 資格已丢失，重新处置只能明确拒绝，不能使用旧 PGID 猜身份。
        let result = child.cleanup();
        drop(child);
        result
    });
    unrelated.assert_live();
    unrelated.cleanup();
    unrelated.assert_reaped();
    assert!(
        transferred,
        "cleanup ECHILD must return the original Child owner"
    );
    assert!(matches!(
        disposition,
        Some(Err(ChildError::Unsupported(
            "child ownership lost; refusing numeric process-group cleanup"
        )))
    ));
    match failure {
        ScanWorkerFailure::Protocol {
            source,
            cleanup: Some(cleanup),
        } => {
            assert_eq!(source.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                source.to_string(),
                "expected one execution Request version 2"
            );
            assert_eq!(
                cleanup.native_io_error().unwrap().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
        other => panic!("lost protocol primary or actual cleanup error: {other:?}"),
    }
}

#[test]
fn checkpoint_error_retains_original_box_and_errno_after_successful_explicit_disposition() {
    let mut fixture = OwnedFailureFixture::new();
    let request = fixture.request();
    let mut child = Some(fixture.take_child());
    let mut driver =
        ScanWorkerDriver::new(&mut child, request, (TARGET, PIN), fixture.deadline, 4096).unwrap();
    assert!(
        child.is_none(),
        "successful configuration transferred original owner"
    );
    let original = Box::new(
        std::fs::File::open("/dev/null/diskgraph-definitely-not-a-directory").unwrap_err(),
    );
    let pointer = (&*original) as *const io::Error;
    let kind = original.kind();
    let errno = original.raw_os_error();
    let mut original = Some(original);
    let error = driver
        .poll(false, || {
            Err::<(), _>(
                original
                    .take()
                    .expect("original checkpoint is consumed once"),
            )
        })
        .unwrap_err();
    let (error, owner) = driver.into_owned_failure(error).into_parts();
    if let Some(mut unexpected) = owner {
        let _ = unexpected.cleanup();
        panic!("already reaped child must not be retained");
    }
    fixture.assert_reaped();
    match error {
        ScanWorkerFailure::Checkpoint { primary, cleanup } => {
            assert_eq!((&*primary) as *const io::Error, pointer);
            assert_eq!(primary.kind(), kind);
            assert_eq!(primary.raw_os_error(), errno);
            assert!(cleanup.is_none());
        }
        other => panic!("lost original boxed error: {other:?}"),
    }
}

#[test]
fn failed_explicit_disposition_preserves_original_primary_and_first_cleanup_object() {
    let mut fixture = OwnedFailureFixture::new();
    let request = fixture.request();
    fixture.external_reap();
    let mut child = Some(fixture.take_child());
    let mut driver =
        ScanWorkerDriver::new(&mut child, request, (TARGET, PIN), fixture.deadline, 4096).unwrap();
    assert!(
        child.is_none(),
        "successful configuration transferred original owner"
    );
    let original = Box::new(
        std::fs::File::open("/dev/null/diskgraph-definitely-not-a-directory").unwrap_err(),
    );
    let pointer = (&*original) as *const io::Error;
    let kind = original.kind();
    let errno = original.raw_os_error();
    let mut original = Some(original);
    let error = driver
        .poll(false, || {
            Err::<(), _>(
                original
                    .take()
                    .expect("original checkpoint is consumed once"),
            )
        })
        .unwrap_err();
    let first_cleanup_pointer = match &error {
        ScanWorkerFailure::Checkpoint {
            cleanup: Some(cleanup),
            ..
        } => {
            let source = cleanup.native_io_error().unwrap();
            assert_eq!(source.raw_os_error(), Some(libc::ECHILD));
            source as *const io::Error
        }
        other => panic!("actual external reap was not observed: {other:?}"),
    };
    let (error, owner) = driver.into_owned_failure(error).into_parts();
    let transferred = owner.is_some();
    if let Some(mut child) = owner {
        // cleanup 的已失权缓存不是新正常退出许可；真实外部 wait 事实已在上面独立见证。
        let _ = child.cleanup();
        drop(child);
    }
    assert!(
        transferred,
        "second cleanup failure cannot discard the original owner"
    );
    match error {
        ScanWorkerFailure::Checkpoint {
            primary,
            cleanup:
                Some(ChildError::Cleanup {
                    primary: first,
                    cleanup: second,
                }),
        } => {
            assert_eq!((&*primary) as *const io::Error, pointer);
            assert_eq!(primary.kind(), kind);
            assert_eq!(primary.raw_os_error(), errno);
            assert_eq!(
                first.native_io_error().unwrap() as *const io::Error,
                first_cleanup_pointer
            );
            assert_eq!(
                first.native_io_error().unwrap().raw_os_error(),
                Some(libc::ECHILD)
            );
            assert!(matches!(
                *second,
                ChildError::Unsupported(
                    "child ownership lost; refusing numeric process-group cleanup"
                )
            ));
        }
        other => panic!("explicit disposition replaced original failure or cleanup: {other:?}"),
    }
}

#[test]
fn rejected_driver_configuration_keeps_original_live_owner_in_caller_slot() {
    let mut fixture = OwnedFailureFixture::new();
    let mut owner = Some(fixture.take_child());
    let rejected = ScanWorkerDriver::new(
        &mut owner,
        WorkerRequest::Cancel {},
        (TARGET, PIN),
        fixture.deadline,
        4096,
    );
    let failure = match rejected {
        Err(failure) => failure,
        Ok(driver) => {
            drop(driver);
            panic!("Cancel cannot initialize a driver");
        }
    };
    let still_owned = owner.is_some();
    let live = owner.as_mut().unwrap().poll();
    let (failure, retained) =
        ScanWorkerOwnedFailure::dispose(failure, owner.take().unwrap()).into_parts();
    if let Some(mut retained) = retained {
        let _ = retained.cleanup();
        panic!("original owner was not actually reaped");
    }
    fixture.assert_reaped();
    assert!(still_owned);
    assert!(
        !live.unwrap(),
        "configuration failure must not implicitly stop the original owner"
    );
    assert!(matches!(
        failure,
        ScanWorkerFailure::Protocol { cleanup: None, .. }
    ));
}

#[test]
fn valid_configuration_without_an_owner_returns_a_fixed_error() {
    let root = tempfile::tempdir().unwrap();
    let request = WorkerRequest::scan(
        root.path(),
        &Default::default(),
        diskgraph_scan_worker::ProtocolLimits {
            max_frame_bytes: 65536,
            max_stream_bytes: 1048576,
            max_nodes: 4096,
            max_depth: 64,
        },
    )
    .unwrap();
    let mut owner = None;
    let result = ScanWorkerDriver::new(
        &mut owner,
        request,
        (TARGET, PIN),
        std::time::Instant::now() + std::time::Duration::from_secs(20),
        4096,
    );
    assert!(owner.is_none());
    match result.err().expect("missing owner must be rejected") {
        ScanWorkerFailure::Protocol { source, cleanup } => {
            assert_eq!(source.kind(), io::ErrorKind::InvalidData);
            assert_eq!(source.to_string(), "scan worker owner missing");
            assert!(cleanup.is_none());
        }
        other => panic!("missing owner classification changed: {other:?}"),
    }
}
