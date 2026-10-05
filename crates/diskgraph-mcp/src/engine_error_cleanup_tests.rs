//! PF-06 清理失败不改变 MCP JSON-RPC 业务码；来源：实际 business_of 与 error_reply。
//! 真实目录清理 I/O 只验证诊断传播，不证明 Child 终止、授权或原生回收。
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use diskgraph_store::StoreError;
use serde_json::json;
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
    let id = json!("cleanup-request");
    let plain =
        crate::error_reply::tool_error(&id, crate::business_of(&primary), &primary.to_string());
    assert_eq!(plain["error"]["data"]["business_code"], code);
    assert_eq!(plain["error"]["data"]["exit_code"], exit);
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
    let frame = crate::error_reply::tool_error(&id, crate::business_of(&error), &error.to_string());
    assert_eq!(frame["jsonrpc"], "2.0");
    assert_eq!(frame["id"], id);
    assert_eq!(frame["error"]["code"], -32001);
    assert_eq!(frame["error"]["data"]["business_code"], code);
    assert_eq!(frame["error"]["data"]["exit_code"], exit);
    assert!(frame.get("result").is_none());
    let message = frame["error"]["message"].as_str().unwrap();
    assert!(message.contains(&first_message));
    assert!(message.contains(&second_message));
}

#[test]
fn mcp_cleanup_keeps_business_codes_and_jsonrpc_association() {
    for (business, code, exit) in [
        (BusinessError::PermissionDenied, "permission_denied", 3),
        (BusinessError::BudgetExceeded, "budget_exceeded", 7),
        (BusinessError::Conflict, "conflict", 9),
    ] {
        assert_wire(EngineError::Business(business), code, exit);
    }
}

#[test]
fn mcp_cleanup_keeps_existing_store_failure_codes() {
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
fn mcp_cleanup_does_not_classify_io_from_business_like_text() {
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
