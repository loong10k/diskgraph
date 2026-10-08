//! 目录版本复用完整原生身份的回归，不把合成字段当成真实文件系统验收。
use super::GitDirectoryVersion;
use crate::live_evidence::git_indexed_directory::GitIndexedDirectory;
use crate::windows_file_state::WindowsFileState;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_BASIC_INFO, FILE_ID_INFO, FILE_STANDARD_INFO,
};

fn state(id: [u8; 16], attributes: u32, directory: bool, deleted: bool) -> WindowsFileState {
    let mut identity = FILE_ID_INFO {
        VolumeSerialNumber: 19,
        ..Default::default()
    };
    identity.FileId.Identifier = id;
    WindowsFileState::from_native_information(
        &identity,
        &FILE_BASIC_INFO {
            CreationTime: 12345,
            FileAttributes: attributes,
            ..Default::default()
        },
        &FILE_STANDARD_INFO {
            Directory: directory,
            DeletePending: deleted,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn captured_directory_identity_keeps_high_bits_and_creation_time() {
    let mut id = [0; 16];
    id[15] = 0xa5;
    let version =
        GitDirectoryVersion::from_windows_state(state(id, FILE_ATTRIBUTE_DIRECTORY, true, false))
            .unwrap();
    assert_eq!(version.identity, (19, id));
    let mut indexed = GitIndexedDirectory {
        windows: (19, id, 12345),
    };
    assert!(version.matches_indexed(&indexed));
    indexed.windows.1[15] ^= 1;
    assert!(!version.matches_indexed(&indexed));
    indexed.windows.1 = id;
    indexed.windows.2 += 1;
    assert!(!version.matches_indexed(&indexed));
    let other = GitDirectoryVersion::from_windows_state(state(
        [0; 16],
        FILE_ATTRIBUTE_DIRECTORY,
        true,
        false,
    ))
    .unwrap();
    assert!(!version.same_identity(&other));
}

#[test]
fn captured_directory_projection_still_rejects_unsafe_states() {
    for (attributes, directory, deleted) in [
        (
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
            true,
            false,
        ),
        (
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_OFFLINE,
            true,
            false,
        ),
        (FILE_ATTRIBUTE_DIRECTORY, true, true),
        (0, false, false),
    ] {
        assert!(
            GitDirectoryVersion::from_windows_state(state([1; 16], attributes, directory, deleted))
                .is_err()
        );
    }
}

#[test]
fn real_directory_version_agrees_with_original_native_capture() {
    let directory = tempfile::tempdir().unwrap();
    let mut budget = crate::live_evidence::probe_budget::ProbeBudget::new(
        &crate::live_evidence::ProbeLimits::default(),
    )
    .unwrap();
    let lease = crate::live_evidence::git_directory_lease::GitDirectoryLease::open(
        directory.path(),
        &mut budget,
    )
    .unwrap();
    let original = WindowsFileState::capture(lease.leaf_file()).unwrap();
    let version = GitDirectoryVersion::capture(lease.leaf_file()).unwrap();
    assert_eq!(version.state, original);
    let observed = original.observation(0, 0, diskgraph_core::WindowsTreeAlignment::Matched);
    assert_eq!(version.identity, (observed.volume, observed.file_id));
    assert!(version.matches_indexed(&GitIndexedDirectory {
        windows: (observed.volume, observed.file_id, observed.creation_time),
    }));
}
