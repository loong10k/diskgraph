use super::*;
use crate::source_evidence::capture_source;

#[test]
fn native_file_operations_refuse_without_verified_source_handles() {
    let workspace = tempfile::tempdir().unwrap();
    let path = workspace.path().join("source.bin");
    std::fs::write(&path, b"unchanged").unwrap();
    assert!(matches!(
        capture_source(&path, 1024),
        Err(OpsError::Stale(message)) if message.contains("unsupported")
    ));
    assert_eq!(std::fs::read(path).unwrap(), b"unchanged");
}
