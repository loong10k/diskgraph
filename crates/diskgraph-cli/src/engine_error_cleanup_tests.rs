//! PF-06 清理失败不改变 CLI 原错误码；来源：实际 error_reply 与命令目录错误契约。
//! 非空目录清理 I/O 是真实错误容器资格，不验证原生 Child 物理回收。
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use std::io;

fn cleanup_error() -> (tempfile::TempDir, io::Error) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("occupied"), b"fixture").unwrap();
    let error = std::fs::remove_dir(directory.path()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::DirectoryNotEmpty);
    assert!(error.raw_os_error().is_some());
    (directory, error)
}

fn assert_wire(primary: EngineError, code: &str, exit: u8) {
    let plain: serde_json::Value =
        serde_json::from_str(&crate::error_reply::line(&primary)).unwrap();
    assert_eq!(plain["error"]["code"], code);
    assert_eq!(plain["error"]["exit_code"], exit);
    let (_first, first) = cleanup_error();
    let (_second, second) = cleanup_error();
    let first_message = first.to_string();
    let second_message = second.to_string();
    let error = EngineError::WithCleanup {
        primary: Box::new(EngineError::WithCleanup {
            primary: Box::new(primary),
            cleanup: first,
        }),
        cleanup: second,
    };
    let frame: serde_json::Value = serde_json::from_str(&crate::error_reply::line(&error)).unwrap();
    assert_eq!(frame["api_version"], 2);
    assert_eq!(frame["ok"], false);
    assert_eq!(frame["error"]["code"], code);
    assert_eq!(frame["error"]["exit_code"], exit);
    assert!(frame.get("data").is_none());
    let message = frame["error"]["message"].as_str().unwrap();
    assert!(message.contains(&first_message));
    assert!(message.contains(&second_message));
}

#[test]
fn cli_cleanup_keeps_business_codes_and_exit_codes() {
    for (business, code, exit) in [
        (BusinessError::PermissionDenied, "permission_denied", 3),
        (BusinessError::BudgetExceeded, "budget_exceeded", 7),
        (BusinessError::Conflict, "conflict", 9),
        (
            BusinessError::RecoveryUnconfirmed,
            "recovery_unconfirmed",
            8,
        ),
    ] {
        assert_wire(EngineError::Business(business), code, exit);
    }
}

#[test]
fn cli_cleanup_keeps_existing_store_failure_codes() {
    for (store, code, exit) in [
        (StoreError::StaleOwner, "conflict", 9),
        (StoreError::BudgetExceeded, "budget_exceeded", 7),
        (
            StoreError::RevisionNotFound("fixture".into()),
            "not_found",
            4,
        ),
    ] {
        assert_wire(EngineError::Store(store), code, exit);
    }
}

#[test]
fn cli_cleanup_does_not_classify_io_from_business_like_text() {
    assert_wire(
        EngineError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "permission_denied",
        )),
        "internal_error",
        10,
    );
    assert_wire(
        EngineError::Io(io::Error::new(
            io::ErrorKind::Interrupted,
            "budget_exceeded",
        )),
        "internal_error",
        10,
    );
}
