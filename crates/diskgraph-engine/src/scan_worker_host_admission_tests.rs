//! 公开宿主材料构造沿用原期限/错误；不把材料构造成功计为进程执行资格。
use crate::{EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget};
use diskgraph_core::BusinessError;
use diskgraph_scan_worker::ProtocolLimits;
use std::fs::File;
use std::time::{Duration, Instant};

fn materials() -> (
    tempfile::TempDir,
    File,
    ScanWorkerHostConfig,
    ScanWorkerRuntimeBudget,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image");
    std::fs::write(&path, b"x").unwrap();
    let config = ScanWorkerHostConfig::from_expected_image([0; 32], 1).unwrap();
    let runtime = ScanWorkerRuntimeBudget::new(
        ProtocolLimits {
            max_frame_bytes: 64 << 10,
            max_stream_bytes: 8 << 20,
            max_nodes: 1000,
            max_depth: 32,
        },
        0,
        1,
    )
    .unwrap();
    (directory, File::open(path).unwrap(), config, runtime)
}

#[test]
fn expired_original_admission_is_not_replaced_by_the_default_new_window() {
    let (_directory, image, expected, runtime) = materials();
    let mut checks = 0;
    assert!(matches!(
        ScanWorkerHost::new_until(
            image,
            expected,
            runtime,
            Instant::now() - Duration::from_secs(1),
            &mut || {
                checks += 1;
                Ok(())
            }
        ),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert_eq!(checks, 1);
}

#[test]
fn original_checkpoint_error_precedes_metadata_or_platform_material_validation() {
    let (_directory, image, expected, runtime) = materials();
    let mut checks = 0;
    assert!(matches!(
        ScanWorkerHost::new_until(
            image,
            expected,
            runtime,
            Instant::now() + Duration::from_secs(5),
            &mut || {
                checks += 1;
                Err(EngineError::Poisoned)
            }
        ),
        Err(EngineError::Poisoned)
    ));
    assert_eq!(checks, 1);
}
