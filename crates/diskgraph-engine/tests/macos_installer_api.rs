//! 外部可信安装程序公开合同；普通 UID 真实调用，不写产品目录。
#![cfg(target_os = "macos")]

use diskgraph_core::BusinessError;
use diskgraph_engine::{EngineError, MacosInstallationPublisher, ScanWorkerHostConfig};
use ed25519_dalek::SigningKey;
use std::time::{Duration, Instant};

fn invoke_all(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> [Result<(), EngineError>; 3] {
    let source = tempfile::tempfile().unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image([1; 32], 1).unwrap();
    [
        MacosInstallationPublisher::prepare(deadline, checkpoint),
        MacosInstallationPublisher::publish(
            &source,
            &expected,
            &SigningKey::from_bytes(&[8; 32]),
            1,
            [1; 16],
            deadline,
            checkpoint,
        ),
        MacosInstallationPublisher::recover(deadline, checkpoint),
    ]
}

#[test]
fn external_installer_rejects_ordinary_identity_before_fixed_layout_writes() {
    assert_ne!(unsafe { libc::getuid() }, 0);
    assert_eq!(unsafe { libc::getuid() }, unsafe { libc::geteuid() });
    for result in invoke_all(Instant::now() + Duration::from_secs(10), &mut || Ok(())) {
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::Unsupported))
        ));
    }
}

#[test]
fn external_installer_preserves_original_checkpoint_error() {
    for result in invoke_all(Instant::now() + Duration::from_secs(10), &mut || {
        Err(EngineError::Poisoned)
    }) {
        assert!(matches!(result, Err(EngineError::Poisoned)));
    }
}

#[test]
fn external_installer_refuses_expired_deadline_before_identity_or_paths() {
    for result in invoke_all(Instant::now() - Duration::from_secs(1), &mut || Ok(())) {
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
    }
}
