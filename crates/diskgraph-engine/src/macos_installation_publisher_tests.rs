//! 普通用户隔离文件测试只证明发行原语，不冒充root安装资格。
use crate::macos_installation_files::MacosInstallationFiles;
use crate::macos_installation_publisher::MacosInstallationPublisher;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use ed25519_dalek::SigningKey;
use std::fs::File;
use std::time::{Duration, Instant};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[test]
fn root_only_publisher_preserves_original_cancellation_before_privilege_or_paths() {
    let source = tempfile::tempfile().unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image([1; 32], 1).unwrap();
    let result = MacosInstallationPublisher::publish(
        &source,
        &expected,
        &SigningKey::from_bytes(&[8; 32]),
        1,
        [1; 16],
        deadline(),
        &mut || Err(EngineError::Poisoned),
    );
    assert!(matches!(result, Err(EngineError::Poisoned)));
}

#[test]
fn ordinary_caller_cannot_reach_fixed_installation_writes() {
    // 本测试必须由ordinary UID执行；root并非等价资格，不能静默跳过。
    assert_ne!(unsafe { libc::getuid() }, 0);
    let source = tempfile::tempfile().unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image([1; 32], 1).unwrap();
    let result = MacosInstallationPublisher::publish(
        &source,
        &expected,
        &SigningKey::from_bytes(&[8; 32]),
        1,
        [1; 16],
        deadline(),
        &mut || Ok(()),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Unsupported))
    ));
}

#[test]
fn fresh_file_creation_never_reuses_or_follows_existing_inode() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let parent = File::open(directory.path()).unwrap();
    MacosInstallationFiles::write_fresh(
        &parent,
        c"material",
        b"original",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    assert!(
        MacosInstallationFiles::write_fresh(
            &parent,
            c"material",
            b"replacement",
            0o444,
            deadline(),
            &mut || Ok(())
        )
        .is_err()
    );
    symlink(
        directory.path().join("material"),
        directory.path().join("alias"),
    )
    .unwrap();
    assert!(
        MacosInstallationFiles::write_fresh(
            &parent,
            c"alias",
            b"replacement",
            0o444,
            deadline(),
            &mut || Ok(())
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(directory.path().join("material")).unwrap(),
        b"original"
    );
}

#[test]
fn failure_after_floor_rename_keeps_new_floor_and_old_active_without_rollback() {
    let directory = tempfile::tempdir().unwrap();
    let base = File::open(directory.path()).unwrap();
    let stage = MacosInstallationFiles::create_directory(&base, c"version").unwrap();
    std::fs::write(directory.path().join("active.json"), b"old-active").unwrap();
    std::fs::write(directory.path().join("epoch-floor.json"), b"old-floor").unwrap();
    MacosInstallationFiles::write_fresh(
        &stage,
        c"active.pending.json",
        b"new-active",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    let anchor = MacosInstallationFiles::write_fresh(
        &stage,
        c"epoch-floor.pending.json",
        b"new-floor",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    let result =
        MacosInstallationFiles::publish_pair(&base, &stage, &anchor, deadline(), &mut || {
            if std::fs::read(directory.path().join("epoch-floor.json")).unwrap() == b"new-floor" {
                Err(EngineError::Poisoned)
            } else {
                Ok(())
            }
        });
    assert!(matches!(result, Err(EngineError::Poisoned)));
    assert_eq!(
        std::fs::read(directory.path().join("epoch-floor.json")).unwrap(),
        b"new-floor"
    );
    assert_eq!(
        std::fs::read(directory.path().join("active.json")).unwrap(),
        b"old-active"
    );
    assert!(
        directory
            .path()
            .join("version/active.pending.json")
            .exists()
    );
}

#[test]
fn real_two_phase_publication_consumes_only_new_staging_names() {
    let directory = tempfile::tempdir().unwrap();
    let base = File::open(directory.path()).unwrap();
    let stage = MacosInstallationFiles::create_directory(&base, c"version").unwrap();
    MacosInstallationFiles::write_fresh(
        &stage,
        c"active.pending.json",
        b"active",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    let anchor = MacosInstallationFiles::write_fresh(
        &stage,
        c"epoch-floor.pending.json",
        b"floor",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    MacosInstallationFiles::publish_pair(&base, &stage, &anchor, deadline(), &mut || Ok(()))
        .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("epoch-floor.json")).unwrap(),
        b"floor"
    );
    assert_eq!(
        std::fs::read(directory.path().join("active.json")).unwrap(),
        b"active"
    );
    assert!(
        !directory
            .path()
            .join("version/active.pending.json")
            .exists()
    );
    assert!(
        !directory
            .path()
            .join("version/epoch-floor.pending.json")
            .exists()
    );
}

#[test]
fn previous_independent_floor_cannot_be_downgraded_rebuilt_or_partially_ignored() {
    use crate::macos_host_settings::MacosHostSettings;
    let active = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"public_key":SigningKey::from_bytes(&[8;32]).verifying_key().to_bytes(),
        "active_epoch":9,"epoch_floor":9,"expected_sha256":vec![1_u8;32],"expected_bytes":100,
        "installation_root":b"/Library/DiskGraph".to_vec(),
        "receipt_path":b"/Library/DiskGraph/v9/receipt.json".to_vec()
    }))
    .unwrap();
    let binding = MacosHostSettings::decode(&active)
        .unwrap()
        .binding_digest()
        .unwrap();
    let floor = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"epoch":9,"active_sha256":binding
    }))
    .unwrap();
    MacosInstallationPublisher::validate_previous(None, None, 1).unwrap();
    MacosInstallationPublisher::validate_previous(Some(&floor), Some(&active), 10).unwrap();
    for epoch in [0, 8, 9] {
        assert!(
            MacosInstallationPublisher::validate_previous(Some(&floor), Some(&active), epoch)
                .is_err()
        );
    }
    for (old_floor, old_active) in [
        (None, Some(active.as_slice())),
        (Some(floor.as_slice()), None),
        (Some(b"broken".as_slice()), Some(active.as_slice())),
        (Some(floor.as_slice()), Some(b"broken".as_slice())),
    ] {
        assert!(MacosInstallationPublisher::validate_previous(old_floor, old_active, 10).is_err());
    }
    let interrupted = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"epoch":10,"active_sha256":binding
    }))
    .unwrap();
    assert!(
        MacosInstallationPublisher::validate_previous(Some(&interrupted), Some(&active), 11)
            .is_err()
    );
}

#[test]
fn held_source_is_copied_in_bounded_chunks_without_changing_its_offset() {
    use sha2::{Digest, Sha256};
    use std::io::{Seek, Write};
    use std::os::unix::fs::MetadataExt;
    let bytes = vec![0x5a_u8; 131_073];
    let mut source = tempfile::tempfile().unwrap();
    source.write_all(&bytes).unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(
        Sha256::digest(&bytes).into(),
        bytes.len() as u64,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let parent = File::open(directory.path()).unwrap();
    let image = MacosInstallationPublisher::copy_for_test(
        &source,
        &expected,
        &parent,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(source.stream_position().unwrap(), bytes.len() as u64);
    assert_eq!(
        std::fs::read(directory.path().join("diskgraph-scan-worker")).unwrap(),
        bytes
    );
    assert_eq!(image.metadata().unwrap().mode() & 0o777, 0o555);
    assert!(
        MacosInstallationPublisher::copy_for_test(
            &source,
            &expected,
            &parent,
            deadline(),
            &mut || Ok(())
        )
        .is_err()
    );
}

#[test]
fn wrong_independent_hash_never_creates_receipt_or_active_material() {
    use std::io::Write;
    let mut source = tempfile::tempfile().unwrap();
    source.write_all(b"actual-image").unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image([0; 32], 12).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let parent = File::open(directory.path()).unwrap();
    let result = MacosInstallationPublisher::copy_for_test(
        &source,
        &expected,
        &parent,
        deadline(),
        &mut || Ok(()),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    assert!(!directory.path().join("receipt.json").exists());
    assert!(!directory.path().join("active.pending.json").exists());
}

#[test]
fn interrupted_pair_can_finish_active_without_replacing_the_committed_floor() {
    use std::os::unix::fs::MetadataExt;
    let directory = tempfile::tempdir().unwrap();
    let base = File::open(directory.path()).unwrap();
    let stage = MacosInstallationFiles::create_directory(&base, c"version").unwrap();
    let anchor = MacosInstallationFiles::write_fresh(
        &base,
        c"epoch-floor.json",
        b"committed-floor",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    let floor_inode = anchor.metadata().unwrap().ino();
    std::fs::write(directory.path().join("active.json"), b"old-active").unwrap();
    MacosInstallationFiles::write_fresh(
        &stage,
        c"active.pending.json",
        b"bound-active",
        0o444,
        deadline(),
        &mut || Ok(()),
    )
    .unwrap();
    MacosInstallationFiles::complete_active(&base, &stage, &anchor, deadline(), &mut || Ok(()))
        .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("active.json")).unwrap(),
        b"bound-active"
    );
    assert_eq!(
        std::fs::read(directory.path().join("epoch-floor.json")).unwrap(),
        b"committed-floor"
    );
    assert_eq!(
        std::fs::metadata(directory.path().join("epoch-floor.json"))
            .unwrap()
            .ino(),
        floor_inode
    );
}

fn recovery_material() -> (Vec<u8>, Vec<u8>) {
    use crate::macos_host_settings::MacosHostSettings;
    let active = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"public_key":SigningKey::from_bytes(&[8;32]).verifying_key().to_bytes(),
        "active_epoch":9,"epoch_floor":9,"expected_sha256":vec![1_u8;32],"expected_bytes":100,
        "installation_root":b"/Library/Application Support/DiskGraph/scan-worker/versions".to_vec(),
        "receipt_path":b"/Library/Application Support/DiskGraph/scan-worker/versions/epoch-00000000000000000009/receipt.json".to_vec()
    })).unwrap();
    let binding = MacosHostSettings::decode(&active)
        .unwrap()
        .binding_digest()
        .unwrap();
    let floor = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"epoch":9,"active_sha256":binding
    }))
    .unwrap();
    (floor, active)
}

#[test]
fn recovery_only_accepts_exact_floor_bound_pending_without_rebuilding_corrupt_floor() {
    let (floor, active) = recovery_material();
    MacosInstallationPublisher::validate_recovery(&floor, &active).unwrap();
    assert!(MacosInstallationPublisher::validate_recovery(b"broken", &active).is_err());
    for (name, replacement) in [
        ("expected_sha256", serde_json::json!(vec![2_u8; 32])),
        ("expected_bytes", serde_json::json!(101)),
        (
            "receipt_path",
            serde_json::json!(
                b"/Library/Application Support/DiskGraph/scan-worker/versions/old/receipt.json"
                    .to_vec()
            ),
        ),
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&active).unwrap();
        changed[name] = replacement;
        assert!(
            MacosInstallationPublisher::validate_recovery(
                &floor,
                &serde_json::to_vec(&changed).unwrap()
            )
            .is_err()
        );
    }
}

#[test]
fn recovery_preserves_cancellation_before_touching_system_paths() {
    let result =
        MacosInstallationPublisher::recover(deadline(), &mut || Err(EngineError::Poisoned));
    assert!(matches!(result, Err(EngineError::Poisoned)));
}
