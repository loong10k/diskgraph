use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_installation_claims::MacosInstallationClaims;
use crate::macos_installation_lease::MacosInstallationLease;
use crate::macos_installation_trust::MacosInstallationTrust;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[82; 32])
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

fn image_material() -> (MacosInstallationClaims, ScanWorkerHostConfig) {
    let path = Path::new("/usr/bin/true");
    let file = File::open(path).unwrap();
    let state = MacosFilesystemState::capture(&file, false).unwrap();
    let digest: [u8; 32] = Sha256::digest(fs::read(path).unwrap()).into();
    let expected = ScanWorkerHostConfig::from_expected_image(digest, state.len).unwrap();
    let claims = MacosInstallationClaims {
        schema_version: 1,
        installation_id: [8; 16],
        epoch: 9,
        policy_version: 1,
        target: env!("DISKGRAPH_ENGINE_TARGET").into(),
        protocol_version: 2,
        pinned_scanner_revision: "158f9cc2f0b332194a3ffc5acec47760c99146d8".into(),
        image_sha256: digest,
        image_bytes: state.len,
        native_path: path.as_os_str().as_bytes().to_vec(),
        volume_uuid: state.volume_uuid,
        fsid: state.fsid,
        device: state.device,
        inode: state.inode,
        birth_seconds: state.birth_seconds,
        birth_nanoseconds: state.birth_nanoseconds,
        fresh_from_birth: true,
    };
    (claims, expected)
}

fn sign(claims: &MacosInstallationClaims) -> Vec<u8> {
    let signature = signing_key()
        .sign(&claims.signing_message())
        .to_bytes()
        .to_vec();
    serde_json::to_vec(&serde_json::json!({"claims": claims, "signature": signature})).unwrap()
}

fn trust(root: &Path) -> MacosInstallationTrust {
    MacosInstallationTrust::from_host(signing_key().verifying_key().to_bytes(), 9, root).unwrap()
}

#[test]
fn system_image_admits_and_revalidates_original_fds_without_claiming_fresh_installation() {
    let (claims, expected) = image_material();
    let mut checkpoints = 0;
    let lease = MacosInstallationLease::admit(
        &sign(&claims),
        &trust(Path::new("/usr")),
        &expected,
        deadline(),
        &mut || {
            checkpoints += 1;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(lease.native_path().as_bytes(), b"/usr/bin/true");
    assert!(checkpoints > 4);
    lease.revalidate(deadline(), &mut || Ok(())).unwrap();
    // 此测试种子的 fresh 声明仅供 verifier/unit fixture，不是可信 root fresh issuer 验收。
}

#[test]
fn original_checkpoint_error_and_expired_deadline_are_preserved() {
    let (claims, expected) = image_material();
    let result = MacosInstallationLease::admit(
        &sign(&claims),
        &trust(Path::new("/usr")),
        &expected,
        deadline(),
        &mut || {
            Err(EngineError::Io(std::io::Error::from_raw_os_error(
                libc::ECANCELED,
            )))
        },
    );
    assert!(
        matches!(result, Err(EngineError::Io(error)) if error.raw_os_error() == Some(libc::ECANCELED))
    );
    let result = MacosInstallationLease::admit(
        &sign(&claims),
        &trust(Path::new("/usr")),
        &expected,
        Instant::now() - Duration::from_secs(1),
        &mut || Ok(()),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::BudgetExceeded))
    ));
}

#[test]
fn real_digest_and_each_native_identity_must_match_signed_claims() {
    let (mut claims, _) = image_material();
    claims.image_sha256[0] ^= 1;
    let expected =
        ScanWorkerHostConfig::from_expected_image(claims.image_sha256, claims.image_bytes).unwrap();
    let result = MacosInstallationLease::admit(
        &sign(&claims),
        &trust(Path::new("/usr")),
        &expected,
        deadline(),
        &mut || Ok(()),
    );
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Conflict))
    ));
    for field in [
        "inode",
        "device",
        "birth_seconds",
        "birth_nanoseconds",
        "fsid",
        "volume_uuid",
    ] {
        let (mut claims, expected) = image_material();
        match field {
            "inode" => claims.inode ^= 1,
            "device" => claims.device ^= 1,
            "birth_seconds" => claims.birth_seconds += 1,
            "birth_nanoseconds" => {
                claims.birth_nanoseconds = (claims.birth_nanoseconds + 1) % 1_000_000_000
            }
            "fsid" => claims.fsid[0] ^= 1,
            "volume_uuid" => claims.volume_uuid[0] ^= 1,
            _ => unreachable!(),
        }
        let result = MacosInstallationLease::admit(
            &sign(&claims),
            &trust(Path::new("/usr")),
            &expected,
            deadline(),
            &mut || Ok(()),
        );
        assert!(
            matches!(result, Err(EngineError::Business(BusinessError::Conflict))),
            "{field}"
        );
    }
}

#[test]
fn checkpoint_cancellation_during_revalidation_does_not_become_success() {
    let (claims, expected) = image_material();
    let receipt = sign(&claims);
    let trusted = trust(Path::new("/usr"));
    let lease =
        MacosInstallationLease::admit(&receipt, &trusted, &expected, deadline(), &mut || Ok(()))
            .unwrap();
    let result = lease.revalidate(deadline(), &mut || Err(EngineError::Poisoned));
    assert!(matches!(result, Err(EngineError::Poisoned)));
}

#[test]
fn signed_non_utf8_missing_path_is_rejected_by_native_lookup_not_unicode_conversion() {
    let (mut claims, expected) = image_material();
    claims.native_path = b"/usr/bin/diskgraph_missing_native_".to_vec();
    claims.native_path.push(255);
    let result = MacosInstallationLease::admit(
        &sign(&claims),
        &trust(Path::new("/usr")),
        &expected,
        deadline(),
        &mut || Ok(()),
    );
    assert!(
        matches!(result, Err(EngineError::Io(error)) if error.raw_os_error() == Some(libc::ENOENT))
    );
}

#[test]
fn signed_private_leaf_cannot_bypass_unsafe_or_user_owned_ancestor() {
    let directory: PathBuf = PathBuf::from("/private/tmp")
        .join(format!("diskgraph_lease_{}_unsafe", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("worker");
    fs::copy("/usr/bin/true", &path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o555)).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o1777)).unwrap();
    let (mut claims, expected) = image_material();
    claims.native_path = path.as_os_str().as_bytes().to_vec();
    // receipt claims身份使用系统镜像也不能绕过更早的祖先安全门禁。
    let result = MacosInstallationLease::admit(
        &sign(&claims),
        &trust(&directory),
        &expected,
        deadline(),
        &mut || Ok(()),
    );
    fs::remove_dir_all(directory).unwrap();
    assert!(matches!(
        result,
        Err(EngineError::Business(BusinessError::Unsupported))
    ));
}
