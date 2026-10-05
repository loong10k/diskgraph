//! PF-06 主错误与清理错误的类型化传播；来源：同一 OpenSpec physical-scan-process。
//! 真实非空目录清理 I/O 只验证错误容器，不证明原生 Child 终止或物理回收。
use crate::EngineError;
use diskgraph_core::BusinessError;
use diskgraph_store::StoreError;
use std::error::Error;
use std::fmt;
use std::io;
use std::sync::Arc;

fn cleanup_error() -> (tempfile::TempDir, io::Error) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("occupied"), b"fixture").unwrap();
    let error = std::fs::remove_dir(directory.path()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::DirectoryNotEmpty);
    assert!(error.raw_os_error().is_some());
    assert!(directory.path().join("occupied").is_file());
    (directory, error)
}

fn wrapped(primary: Box<EngineError>, cleanup: io::Error) -> EngineError {
    EngineError::WithCleanup { primary, cleanup }
}

#[test]
fn plain_primary_is_the_same_borrowed_error_without_cleanup() {
    let errors = [
        EngineError::Business(BusinessError::PermissionDenied),
        EngineError::Store(StoreError::StaleOwner),
        EngineError::Io(io::Error::new(io::ErrorKind::Interrupted, "original I/O")),
        EngineError::Poisoned,
    ];
    for error in &errors {
        assert!(std::ptr::eq(error.primary(), error));
    }
}

#[test]
fn business_primary_and_actual_cleanup_io_remain_separate() {
    let (_keep, cleanup) = cleanup_error();
    let cleanup_kind = cleanup.kind();
    let cleanup_code = cleanup.raw_os_error();
    let cleanup_message = cleanup.to_string();
    let primary = Box::new(EngineError::Business(BusinessError::PermissionDenied));
    let original = primary.as_ref() as *const EngineError;
    let error = wrapped(primary, cleanup);

    assert!(std::ptr::eq(error.primary(), original));
    assert!(matches!(
        error.primary(),
        EngineError::Business(BusinessError::PermissionDenied)
    ));
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<EngineError>()
        .unwrap();
    assert!(std::ptr::eq(source, original));
    assert!(error.to_string().contains("permission_denied"));
    assert!(error.to_string().contains(&cleanup_message));
    let EngineError::WithCleanup { cleanup, .. } = error else {
        panic!("actual cleanup failure must remain available independently");
    };
    assert_eq!(cleanup.kind(), cleanup_kind);
    assert_eq!(cleanup.raw_os_error(), cleanup_code);
    assert_eq!(cleanup.to_string(), cleanup_message);
}

#[test]
fn nested_primary_and_source_chain_keep_actual_sqlite_busy() {
    let database = tempfile::tempdir().unwrap();
    let path = database.path().join("source.sqlite");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute_batch("CREATE TABLE item (value INTEGER); BEGIN IMMEDIATE;")
        .unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    reader.busy_timeout(std::time::Duration::ZERO).unwrap();
    let original_error = reader
        .execute("INSERT INTO item VALUES (1)", [])
        .unwrap_err();
    let original_code = original_error.sqlite_extended_error_code();
    writer.execute_batch("ROLLBACK").unwrap();
    assert_eq!(original_code, Some(5));

    let primary = Box::new(EngineError::Store(StoreError::Sqlite(original_error)));
    let original = primary.as_ref() as *const EngineError;
    let (_first, first_cleanup) = cleanup_error();
    let (_second, second_cleanup) = cleanup_error();
    let inner = Box::new(wrapped(primary, first_cleanup));
    let original_inner = inner.as_ref() as *const EngineError;
    let outer = wrapped(inner, second_cleanup);

    assert!(std::ptr::eq(outer.primary(), original));
    let first = outer
        .source()
        .unwrap()
        .downcast_ref::<EngineError>()
        .unwrap();
    assert!(std::ptr::eq(first, original_inner));
    let second = first
        .source()
        .unwrap()
        .downcast_ref::<EngineError>()
        .unwrap();
    assert!(std::ptr::eq(second, original));
    let EngineError::Store(store) = outer.primary() else {
        panic!("nested cleanup must not rewrite the original Store error");
    };
    assert!(store.is_busy());
    let StoreError::Sqlite(error) = store else {
        panic!("the original SQLite failure must retain its native code");
    };
    assert_eq!(error.sqlite_extended_error_code(), original_code);
}

#[test]
fn nested_io_primary_retains_real_kind_errno_and_message() {
    let missing = tempfile::tempdir().unwrap();
    let original_error = std::fs::File::open(missing.path().join("missing")).unwrap_err();
    let kind = original_error.kind();
    let code = original_error.raw_os_error();
    let message = original_error.to_string();
    assert_eq!(kind, io::ErrorKind::NotFound);
    assert!(code.is_some());
    let primary = Box::new(EngineError::Io(original_error));
    let original = primary.as_ref() as *const EngineError;
    let (_first, first) = cleanup_error();
    let (_second, second) = cleanup_error();
    let error = wrapped(Box::new(wrapped(primary, first)), second);

    assert!(std::ptr::eq(error.primary(), original));
    let EngineError::Io(io) = error.primary() else {
        panic!("I/O primary must remain I/O under nested cleanup");
    };
    assert_eq!(io.kind(), kind);
    assert_eq!(io.raw_os_error(), code);
    assert_eq!(io.to_string(), message);
}

/// 不可克隆的原 I/O payload；来源：PF-06 错误传播对象身份负控，无 Java 对应对象。
#[derive(Debug)]
struct OriginalIoPayload {
    identity: Arc<()>,
}

impl fmt::Display for OriginalIoPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("original typed I/O payload")
    }
}

impl Error for OriginalIoPayload {}

#[test]
fn io_payload_identity_survives_cleanup_without_stringification() {
    let identity = Arc::new(());
    let io = io::Error::new(
        io::ErrorKind::PermissionDenied,
        OriginalIoPayload {
            identity: Arc::clone(&identity),
        },
    );
    let primary = Box::new(EngineError::Io(io));
    let original = primary.as_ref() as *const EngineError;
    let (_keep, cleanup) = cleanup_error();
    let error = wrapped(primary, cleanup);
    assert!(std::ptr::eq(error.primary(), original));
    let EngineError::Io(io) = error.primary() else {
        panic!("typed I/O must not become a Business permission denial");
    };
    assert_eq!(io.kind(), io::ErrorKind::PermissionDenied);
    let payload = io
        .get_ref()
        .unwrap()
        .downcast_ref::<OriginalIoPayload>()
        .unwrap();
    assert!(Arc::ptr_eq(&payload.identity, &identity));
}
