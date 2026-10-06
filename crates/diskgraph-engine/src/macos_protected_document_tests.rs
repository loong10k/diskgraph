use crate::EngineError;
use crate::macos_protected_document::MacosProtectedDocument;
use diskgraph_core::BusinessError;
use std::time::{Duration, Instant};

#[test]
fn actual_root_owned_document_is_read_bounded_without_following_etc_alias() {
    let path = b"/private/etc/hosts";
    let bytes = MacosProtectedDocument::read(
        path,
        64 * 1024,
        Instant::now() + Duration::from_secs(10),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(bytes, std::fs::read("/private/etc/hosts").unwrap());
    assert!(
        MacosProtectedDocument::read(
            b"/etc/hosts",
            64 * 1024,
            Instant::now() + Duration::from_secs(10),
            &mut || Ok(())
        )
        .is_err()
    );
}
#[test]
fn protected_document_cannot_expand_budget_or_accept_user_owned_parent() {
    let result = MacosProtectedDocument::read(
        b"/private/etc/hosts",
        1,
        Instant::now() + Duration::from_secs(10),
        &mut || Ok(()),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("active.json");
    std::fs::write(&path, b"{}").unwrap();
    use std::os::unix::ffi::OsStrExt;
    assert!(
        MacosProtectedDocument::read(
            path.as_os_str().as_bytes(),
            64 * 1024,
            Instant::now() + Duration::from_secs(10),
            &mut || Ok(())
        )
        .is_err()
    );
}
#[test]
fn protected_document_preserves_original_cancellation_before_any_open() {
    assert!(matches!(
        MacosProtectedDocument::read(
            b"/not-present",
            64 * 1024,
            Instant::now() + Duration::from_secs(10),
            &mut || Err(EngineError::Poisoned)
        ),
        Err(EngineError::Poisoned)
    ));
}
