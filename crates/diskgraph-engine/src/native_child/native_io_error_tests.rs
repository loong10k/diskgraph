//! 原 OS 错误对象传播回归；来源：PF-06，不靠字符串恢复 errno。
use super::ChildError;
use std::{error::Error, io};

/// 用于验证原 I/O payload 身份的测试错误；来源：PF-06 原生错误传播回归。
#[derive(Debug, thiserror::Error)]
#[error("original io payload")]
struct Payload;

#[test]
fn actual_failed_file_open_keeps_original_os_error_in_source_chain() {
    let root = tempfile::tempdir().unwrap();
    let original = std::fs::File::open(root.path().join("missing")).unwrap_err();
    let kind = original.kind();
    let code = original.raw_os_error();
    assert!(code.is_some());
    let error = ChildError::io("open native fixture", original);
    let source = error.source().expect("original native I/O source");
    let source = source.downcast_ref::<io::Error>().expect("typed io::Error");
    assert_eq!(source.kind(), kind);
    assert_eq!(source.raw_os_error(), code);
}

#[test]
fn cloned_native_failure_shares_the_original_custom_payload_and_kind() {
    let original = io::Error::new(io::ErrorKind::Interrupted, Payload);
    let error = ChildError::io("custom native fixture", original);
    let clone = error.clone();
    let source = error.source().expect("original source");
    let cloned = clone.source().expect("cloned source");
    assert!(std::ptr::eq(source, cloned));
    let source = source.downcast_ref::<io::Error>().expect("typed io::Error");
    assert_eq!(source.kind(), io::ErrorKind::Interrupted);
    assert_eq!(source.raw_os_error(), None);
    assert!(
        source
            .get_ref()
            .unwrap()
            .downcast_ref::<Payload>()
            .is_some()
    );
}

#[test]
fn native_primary_and_actual_cleanup_errors_remain_separate() {
    let directory = tempfile::tempdir().unwrap();
    let cleanup = std::fs::File::open(directory.path().join("missing")).unwrap_err();
    let cleanup_code = cleanup.raw_os_error();
    let error = ChildError::io(
        "primary",
        io::Error::new(io::ErrorKind::Interrupted, Payload),
    )
    .with_cleanup(Err(ChildError::io("cleanup", cleanup)));
    let clone = error.clone();
    let primary = error.native_io_error().expect("original primary");
    assert_eq!(primary.kind(), io::ErrorKind::Interrupted);
    assert!(std::ptr::eq(primary, clone.native_io_error().unwrap()));
    let ChildError::Cleanup { cleanup, .. } = error else {
        panic!("cleanup diagnostics required");
    };
    let cleanup = cleanup
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(cleanup.raw_os_error(), cleanup_code);
    assert!(cleanup_code.is_some());
}
