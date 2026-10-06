//! Windows镜像材料的真实原句柄、共享冲突及预算回归；不是加载或扫描执行资格验收。
use crate::windows_scan_image_lease::WindowsScanImageLease;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

const BYTES: &[u8] = b"independent native image material";

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, ScanWorkerHostConfig) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.bin");
    std::fs::write(&path, BYTES).unwrap();
    let expected =
        ScanWorkerHostConfig::from_expected_image(Sha256::digest(BYTES).into(), BYTES.len() as u64)
            .unwrap();
    (directory, path, expected)
}

#[test]
fn verified_original_lease_denies_new_writers_and_name_replacement_until_drop() {
    let (_directory, path, expected) = fixture();
    let deadline = Instant::now() + Duration::from_secs(5);
    let lease = WindowsScanImageLease::prepare(
        File::open(&path).unwrap(),
        &expected,
        deadline,
        &mut || Ok(()),
    )
    .unwrap();
    let error = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(32));
    let moved = path.with_extension("moved");
    assert_eq!(
        std::fs::rename(&path, &moved).unwrap_err().raw_os_error(),
        Some(32)
    );
    lease.validate(deadline, &mut || Ok(())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), BYTES);
    drop(lease);
    std::fs::rename(&path, &moved).unwrap();
    OpenOptions::new().write(true).open(moved).unwrap();
}

#[test]
fn independent_wrong_digest_rejects_and_releases_the_new_read_lease() {
    let (_directory, path, _) = fixture();
    let expected = ScanWorkerHostConfig::from_expected_image([0; 32], BYTES.len() as u64).unwrap();
    assert!(matches!(
        WindowsScanImageLease::prepare(
            File::open(&path).unwrap(),
            &expected,
            Instant::now() + Duration::from_secs(5),
            &mut || Ok(())
        ),
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    OpenOptions::new().write(true).open(&path).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), BYTES);
}

#[test]
fn existing_write_handle_prevents_qualification_without_waiting_for_it() {
    let (_directory, path, expected) = fixture();
    let writer = OpenOptions::new().write(true).open(&path).unwrap();
    let result = WindowsScanImageLease::prepare(
        File::open(&path).unwrap(),
        &expected,
        Instant::now() + Duration::from_secs(5),
        &mut || Ok(()),
    );
    assert!(matches!(result, Err(EngineError::Io(ref error)) if error.raw_os_error() == Some(32)));
    drop(writer);
    let lease = WindowsScanImageLease::prepare(
        File::open(&path).unwrap(),
        &expected,
        Instant::now() + Duration::from_secs(5),
        &mut || Ok(()),
    )
    .unwrap();
    drop(lease);
}

#[test]
fn original_expiry_and_checkpoint_failure_do_not_leave_a_read_lease() {
    let (_directory, path, expected) = fixture();
    let mut calls = 0;
    assert!(matches!(
        WindowsScanImageLease::prepare(
            File::open(&path).unwrap(),
            &expected,
            Instant::now() - Duration::from_secs(1),
            &mut || {
                calls += 1;
                Ok(())
            }
        ),
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
    assert_eq!(calls, 1);
    assert!(matches!(
        WindowsScanImageLease::prepare(
            File::open(&path).unwrap(),
            &expected,
            Instant::now() + Duration::from_secs(5),
            &mut || Err(BusinessError::PermissionDenied.into())
        ),
        Err(EngineError::Business(BusinessError::PermissionDenied))
    ));
    OpenOptions::new().write(true).open(path).unwrap();
}

#[test]
fn deployment_attribute_open_returns_the_same_readable_original_file() {
    use std::io::Read;
    let (_directory, path, _) = fixture();
    let mut file = WindowsScanImageLease::open_source(&path).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, BYTES);
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
    drop(file);
    OpenOptions::new().write(true).open(path).unwrap();
}
