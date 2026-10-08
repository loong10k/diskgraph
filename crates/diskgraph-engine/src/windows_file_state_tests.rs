//! 原生信息投影保留安全门禁及完整版本，不使用兼容 64 位身份代替完整 ID。
use crate::windows_file_state::WindowsFileState;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_ID_INFO,
    FILE_STANDARD_INFO,
};

#[test]
fn native_projection_preserves_high_identity_bits_and_version_changes() {
    let mut id = FILE_ID_INFO::default();
    id.FileId.Identifier[15] = 1;
    let mut basic = FILE_BASIC_INFO::default();
    let mut standard = FILE_STANDARD_INFO::default();
    let original = WindowsFileState::from_native_information(&id, &basic, &standard).unwrap();
    assert!(original.legacy_identity().is_none());
    basic.LastAccessTime = 42;
    assert_eq!(
        original,
        WindowsFileState::from_native_information(&id, &basic, &standard).unwrap()
    );
    basic.ChangeTime = 42;
    assert_ne!(
        original,
        WindowsFileState::from_native_information(&id, &basic, &standard).unwrap()
    );
    basic.ChangeTime = 0;
    id.FileId.Identifier[15] = 2;
    assert_ne!(
        original,
        WindowsFileState::from_native_information(&id, &basic, &standard).unwrap()
    );
    id.FileId.Identifier[15] = 1;
    standard.EndOfFile = 1;
    assert_ne!(
        original,
        WindowsFileState::from_native_information(&id, &basic, &standard).unwrap()
    );
}

#[test]
fn native_projection_retains_reparse_placeholder_deletion_and_negative_length_gates() {
    let id = FILE_ID_INFO::default();
    let mut basic = FILE_BASIC_INFO::default();
    let mut standard = FILE_STANDARD_INFO {
        EndOfFile: -1,
        ..FILE_STANDARD_INFO::default()
    };
    assert!(WindowsFileState::from_native_information(&id, &basic, &standard).is_err());
    standard.EndOfFile = 0;
    basic.FileAttributes = FILE_ATTRIBUTE_REPARSE_POINT;
    assert!(
        WindowsFileState::from_native_information(&id, &basic, &standard)
            .unwrap()
            .validate(false)
            .is_err()
    );
    basic.FileAttributes = FILE_ATTRIBUTE_OFFLINE;
    assert!(
        WindowsFileState::from_native_information(&id, &basic, &standard)
            .unwrap()
            .placeholder()
    );
    basic.FileAttributes = 0;
    standard.DeletePending = true;
    assert!(
        WindowsFileState::from_native_information(&id, &basic, &standard)
            .unwrap()
            .validate(false)
            .is_err()
    );
}
