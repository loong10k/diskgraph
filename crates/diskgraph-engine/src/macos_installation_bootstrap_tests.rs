use crate::EngineError;
use crate::macos_installation_bootstrap::MacosInstallationBootstrap;
use diskgraph_core::BusinessError;
use std::time::{Duration, Instant};
#[test]
fn bootstrap_preserves_original_cancel_before_privilege_or_paths() {
    assert!(matches!(
        MacosInstallationBootstrap::prepare(Instant::now() + Duration::from_secs(10), &mut || Err(
            EngineError::Poisoned
        )),
        Err(EngineError::Poisoned)
    ));
}
#[test]
fn ordinary_identity_cannot_create_protected_installation_layout() {
    assert_ne!(
        unsafe { libc::getuid() },
        0,
        "ordinary UID fixture must not run as root"
    );
    assert!(matches!(
        MacosInstallationBootstrap::prepare(Instant::now() + Duration::from_secs(10), &mut || Ok(
            ()
        )),
        Err(EngineError::Business(BusinessError::Unsupported))
    ));
}
#[test]
fn expired_bootstrap_budget_precedes_filesystem_mutation() {
    assert!(matches!(
        MacosInstallationBootstrap::prepare(Instant::now(), &mut || Ok(())),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}
