//! 固定用户 home 下的隔离夹具；不创建产品的全局恢复域或数据库。
use super::{current_home, open_directory};
use crate::TrustedLocalRecoveryDomain;
use crate::recovery_slot::SlotError;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::time::{Duration, Instant};

fn fixture() -> tempfile::TempDir {
    let home = current_home(Instant::now() + Duration::from_secs(5)).unwrap();
    #[cfg(target_os = "linux")]
    let prefix = std::ffi::OsStr::from_bytes(b".diskgraph_recovery_fixture_\xff");
    #[cfg(target_os = "macos")]
    let prefix = std::ffi::OsStr::new(".diskgraph_recovery_fixture_");
    let directory = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(home)
        .unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    directory
}

#[test]
fn fixed_namespace_creation_keeps_actual_home_identity_and_reuses_original_slot() {
    let home = fixture();
    let deadline = Instant::now() + Duration::from_secs(5);
    let directory = open_directory(home.path(), deadline).unwrap();
    let actual = directory.metadata().unwrap();
    let expected = home
        .path()
        .join(".local/state/diskgraph/recovery")
        .metadata()
        .unwrap();
    assert_eq!(
        (actual.dev(), actual.ino()),
        (expected.dev(), expected.ino())
    );
    assert_eq!(actual.mode() & 0o7777, 0o700);
    let domain = TrustedLocalRecoveryDomain::from_host(directory, deadline).unwrap();
    domain
        .reserve(deadline)
        .unwrap()
        .abort_before_birth(deadline)
        .unwrap();
    let reopened = TrustedLocalRecoveryDomain::from_host(
        open_directory(home.path(), deadline).unwrap(),
        deadline,
    )
    .unwrap();
    reopened
        .reserve(deadline)
        .unwrap()
        .abort_before_birth(deadline)
        .unwrap();
}

#[test]
fn linked_namespace_and_existing_unsafe_permissions_are_not_repaired() {
    let home = fixture();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), home.path().join(".local")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    assert!(open_directory(home.path(), deadline).is_err());
    assert!(!outside.path().join("state").exists());
    std::fs::remove_file(home.path().join(".local")).unwrap();
    let directory = open_directory(home.path(), deadline).unwrap();
    let path = home.path().join(".local/state/diskgraph/recovery");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        TrustedLocalRecoveryDomain::from_host(directory, deadline),
        Err(SlotError::Unsupported)
    ));
    assert!(open_directory(home.path(), deadline).is_err());
    assert_eq!(path.metadata().unwrap().mode() & 0o7777, 0o777);
}

#[test]
fn expired_namespace_admission_creates_no_state() {
    let home = fixture();
    assert!(matches!(
        open_directory(home.path(), Instant::now()),
        Err(SlotError::Deadline)
    ));
    assert!(!home.path().join(".local").exists());
}
