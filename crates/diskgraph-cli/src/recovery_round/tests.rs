//! 确定性恢复调度回归；验证公平性和原错误，不冒充原生 owner 清理。
use super::recovery_round;
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use std::cell::RefCell;

#[test]
fn pending_scan_still_polls_probe_in_the_same_round() {
    let visits = RefCell::new(Vec::new());
    let result = recovery_round(
        || {
            visits.borrow_mut().push("seal");
            Ok(())
        },
        || {
            visits.borrow_mut().push("scan");
            Ok(false)
        },
        || {
            visits.borrow_mut().push("probe");
            Ok(true)
        },
    )
    .unwrap();
    assert!(!result);
    assert_eq!(*visits.borrow(), ["seal", "scan", "probe"]);
}

#[test]
fn scan_error_still_polls_probe_and_preserves_original_error() {
    let visits = RefCell::new(Vec::new());
    let result = recovery_round(
        || Ok(()),
        || {
            visits.borrow_mut().push("scan");
            Err(EngineError::Business(BusinessError::Conflict))
        },
        || {
            visits.borrow_mut().push("probe");
            Err(EngineError::Business(BusinessError::Timeout))
        },
    );
    assert_eq!(*visits.borrow(), ["scan", "probe"]);
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Conflict))
    ));
}

#[test]
fn failed_seal_never_starts_drain() {
    let result = recovery_round(
        || Err(EngineError::Business(BusinessError::PermissionDenied)),
        || panic!("unsealed scan must not drain"),
        || panic!("unsealed probe must not drain"),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
}

#[test]
fn probe_pending_or_error_cannot_be_reported_complete() {
    assert!(!recovery_round(|| Ok(()), || Ok(true), || Ok(false)).unwrap());
    assert!(matches!(
        recovery_round(
            || Ok(()),
            || Ok(true),
            || Err(EngineError::Business(BusinessError::Timeout))
        ),
        Err(EngineError::Business(BusinessError::Timeout))
    ));
    assert!(recovery_round(|| Ok(()), || Ok(true), || Ok(true)).unwrap());
}

#[test]
fn probe_error_is_not_hidden_by_pending_scan() {
    let result = recovery_round(
        || Ok(()),
        || Ok(false),
        || Err(EngineError::Business(BusinessError::Timeout)),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Timeout))
    ));
}
