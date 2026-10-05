//! PF-06 清理包装保持旧 FFI 字符串 wire；来源：scan_coordinator::progress_error。
//! 本模块由父 scan_coordinator 以 cfg(test)/path 挂载，不扩大生产函数可见性。
use super::progress_error;
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

#[test]
fn ffi_nested_cleanup_retains_exact_permission_denied_wire() {
    let plain: serde_json::Value = serde_json::from_str(&crate::response(Err(progress_error(
        EngineError::Business(BusinessError::PermissionDenied),
    ))))
    .unwrap();
    assert_eq!(plain["error"], "permission_denied");
    let (_first, first) = cleanup_error();
    let (_second, second) = cleanup_error();
    let error = EngineError::WithCleanup {
        primary: Box::new(EngineError::WithCleanup {
            primary: Box::new(EngineError::Business(BusinessError::PermissionDenied)),
            cleanup: first,
        }),
        cleanup: second,
    };
    let frame: serde_json::Value =
        serde_json::from_str(&crate::response(Err(progress_error(error)))).unwrap();
    assert_eq!(frame, plain);
    assert_eq!(frame["schema_version"], 1);
    assert_eq!(frame["ok"], false);
    assert!(frame.get("data").is_none());
}

fn assert_diagnostic(make_primary: impl Fn() -> EngineError, expected_prefix: &str) {
    let plain = progress_error(make_primary());
    assert!(plain.starts_with(expected_prefix));
    let (_first, first) = cleanup_error();
    let (_second, second) = cleanup_error();
    let first_message = first.to_string();
    let second_message = second.to_string();
    // 夹具显式创建同类 typed 主错误，原 error 不实现 Clone；不从诊断文本推断 variant。
    let primary = make_primary();
    let error = EngineError::WithCleanup {
        primary: Box::new(EngineError::WithCleanup {
            primary: Box::new(primary),
            cleanup: first,
        }),
        cleanup: second,
    };
    let frame: serde_json::Value =
        serde_json::from_str(&crate::response(Err(progress_error(error)))).unwrap();
    assert_eq!(frame["schema_version"], 1);
    assert_eq!(frame["ok"], false);
    assert!(frame.get("data").is_none());
    let message = frame["error"].as_str().unwrap();
    assert!(message.starts_with(expected_prefix));
    assert!(message.contains(&first_message));
    assert!(message.contains(&second_message));
    assert_ne!(message, "permission_denied");
}

#[test]
fn ffi_cleanup_preserves_budget_code_in_the_legacy_string_wire() {
    assert_diagnostic(
        || EngineError::Business(BusinessError::BudgetExceeded),
        "budget_exceeded (exit 7)",
    );
}

#[test]
fn ffi_cleanup_keeps_store_diagnostic_without_new_structured_code() {
    assert_diagnostic(
        || EngineError::Store(StoreError::BudgetExceeded),
        "query response budget exceeded by one record",
    );
}

#[test]
fn ffi_cleanup_does_not_convert_io_kind_to_business_permission_denied() {
    assert_diagnostic(
        || {
            EngineError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "original I/O permission-like diagnostic",
            ))
        },
        "original I/O permission-like diagnostic",
    );
}
