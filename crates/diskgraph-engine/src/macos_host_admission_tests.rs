use crate::{EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget};
use diskgraph_scan_worker::ProtocolLimits;
use std::time::{Duration, Instant};
fn budget() -> ScanWorkerRuntimeBudget {
    ScanWorkerRuntimeBudget::new(
        ProtocolLimits {
            max_frame_bytes: 65536,
            max_stream_bytes: 1048576,
            max_nodes: 1000,
            max_depth: 16,
        },
        65536,
        2,
    )
    .unwrap()
}
#[test]
fn fixed_installation_constructor_preserves_original_cancellation() {
    let result = ScanWorkerHost::from_installed_macos(
        budget(),
        Instant::now() + Duration::from_secs(10),
        &mut || Err(EngineError::Poisoned),
    );
    assert!(matches!(result, Err(EngineError::Poisoned)));
}
#[test]
fn ordinary_file_constructor_never_acquires_mac_execution_qualification() {
    let file = std::fs::File::open("/usr/bin/true").unwrap();
    let length = file.metadata().unwrap().len();
    let host = ScanWorkerHost::new(
        file,
        ScanWorkerHostConfig::from_expected_image([1; 32], length).unwrap(),
        budget(),
    )
    .unwrap();
    let result =
        host.prepare_macos_installation(Instant::now() + Duration::from_secs(10), &mut || Ok(()));
    assert!(matches!(
        result,
        Err(EngineError::Business(
            diskgraph_core::BusinessError::Unsupported
        ))
    ));
}

#[test]
fn running_epoch_check_preserves_cancellation_and_rejects_ordinary_file_host() {
    let file = std::fs::File::open("/usr/bin/true").unwrap();
    let length = file.metadata().unwrap().len();
    let host = ScanWorkerHost::new(
        file,
        ScanWorkerHostConfig::from_expected_image([1; 32], length).unwrap(),
        budget(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    assert!(matches!(
        host.authorize_macos_epoch(deadline, &mut || Err(EngineError::Poisoned)),
        Err(EngineError::Poisoned)
    ));
    assert!(matches!(
        host.authorize_macos_epoch(deadline, &mut || Ok(())),
        Err(EngineError::Business(
            diskgraph_core::BusinessError::Unsupported
        ))
    ));
}
