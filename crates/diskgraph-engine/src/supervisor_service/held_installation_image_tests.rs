use super::held_installation_image::{HeldInstallationImage, verify_digest};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};

#[test]
fn actual_held_image_digest_covers_multiple_bounded_chunks_and_original_deadline() {
    let mut fixture = tempfile::NamedTempFile::new().unwrap();
    let bytes = vec![0x5b; 3 * 65536 + 11];
    fixture.write_all(&bytes).unwrap();
    let expected = hex::encode(Sha256::digest(&bytes));
    verify_digest(
        &mut File::open(fixture.path()).unwrap(),
        &expected,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap();
    let error = verify_digest(
        &mut File::open(fixture.path()).unwrap(),
        &expected,
        Instant::now(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(
        verify_digest(
            &mut File::open(fixture.path()).unwrap(),
            &"0".repeat(64),
            Instant::now() + Duration::from_secs(5)
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::PermissionDenied
    );
}

#[test]
fn ordinary_user_cannot_derive_root_installation_trust_from_own_file() {
    let fixture = tempfile::NamedTempFile::new().unwrap();
    assert!(
        HeldInstallationImage::open(
            fixture.path(),
            &"0".repeat(64),
            Instant::now() + Duration::from_secs(5)
        )
        .is_err()
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires current CLI ELF and root-provisioned isolated /var/lib; mandatory native CI"]
fn root_installer_original_image_accepts_exact_digest_and_rejects_aliases() {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let root = tempfile::Builder::new()
        .prefix("diskgraph-image-qualified-")
        .tempdir_in("/var/lib")
        .unwrap();
    let source =
        std::env::var_os("DG_LINUX_SUPERVISOR_CLI").expect("current actual CLI ELF required");
    let image = root.path().join("diskgraph");
    std::fs::copy(source, &image).unwrap();
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o555)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut original = File::open(&image).unwrap();
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        assert!(Instant::now() < deadline);
        let count = original.read(&mut bytes).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    let expected = hex::encode(digest.finalize());
    let held = HeldInstallationImage::open(&image, &expected, deadline).unwrap();
    assert!(held.into_file().metadata().unwrap().is_file());
    assert!(HeldInstallationImage::open(&image, &"0".repeat(64), deadline).is_err());
    let alias = root.path().join("alias");
    symlink(&image, &alias).unwrap();
    assert!(HeldInstallationImage::open(&alias, &expected, deadline).is_err());
    std::fs::remove_file(&alias).unwrap();
    std::fs::hard_link(&image, &alias).unwrap();
    assert!(HeldInstallationImage::open(&image, &expected, deadline).is_err());
    std::fs::remove_file(&alias).unwrap();
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o775)).unwrap();
    assert!(HeldInstallationImage::open(&image, &expected, deadline).is_err());
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(HeldInstallationImage::open(&image, &expected, deadline).is_err());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        HeldInstallationImage::open(&image, &expected, Instant::now())
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::TimedOut
    );
    // 原File不会按路径重开：安装者原子替换路径后，原held镜像仍是旧对象。
    let held = HeldInstallationImage::open(&image, &expected, deadline)
        .unwrap()
        .into_file();
    let original_identity = (
        held.metadata().unwrap().dev(),
        held.metadata().unwrap().ino(),
    );
    std::fs::rename(&image, root.path().join("original")).unwrap();
    std::fs::write(&image, b"untrusted replacement").unwrap();
    assert_eq!(
        (
            held.metadata().unwrap().dev(),
            held.metadata().unwrap().ino()
        ),
        original_identity
    );
    assert!(HeldInstallationImage::open(&image, &expected, deadline).is_err());
    let config_path = root.path().join("supervisor.json");
    let value = serde_json::json!({
        "schema_version":1, "frontend_uid":1000, "service_uid":23111, "service_gid":23111,
        "state_dir":root.path(), "data_dir":root.path().join("data"), "socket_path":root.path().join("frontend.sock"),
        "cli_path":image, "cli_sha256":expected, "worker_path":root.path().join("worker"), "worker_sha256":"cd".repeat(32)
    });
    std::fs::write(&config_path, serde_json::to_vec(&value).unwrap()).unwrap();
    let config = super::InstallationConfig::open_root(&config_path, deadline).unwrap();
    assert_eq!(config.service_uid, 23111);
    let config_alias = root.path().join("config_alias");
    symlink(&config_path, &config_alias).unwrap();
    assert!(super::InstallationConfig::open_root(&config_alias, deadline).is_err());
    std::fs::write(&config_path, vec![b' '; 4097]).unwrap();
    assert!(super::InstallationConfig::open_root(&config_path, deadline).is_err());
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(super::InstallationConfig::open_root(&config_path, deadline).is_err());
}
