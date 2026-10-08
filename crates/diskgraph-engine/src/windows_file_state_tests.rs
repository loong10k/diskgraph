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

#[test]
fn native_write_time_projection_preserves_epoch_precision_and_unsigned_bits() {
    use std::time::{Duration, SystemTime};
    let epoch_ticks = 116_444_736_000_000_000_u64;
    let file_epoch = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(11_644_473_600))
        .unwrap();
    for ticks in [
        0,
        1,
        epoch_ticks - 1,
        epoch_ticks,
        epoch_ticks + 1,
        epoch_ticks + 12_345_678,
        i64::MAX as u64,
        1_u64 << 63,
        u64::MAX,
    ] {
        let basic = FILE_BASIC_INFO {
            LastWriteTime: ticks as i64,
            ..FILE_BASIC_INFO::default()
        };
        let state = WindowsFileState::from_native_information(
            &FILE_ID_INFO::default(),
            &basic,
            &FILE_STANDARD_INFO::default(),
        )
        .unwrap();
        let expected = file_epoch.checked_add(Duration::new(
            ticks / 10_000_000,
            ((ticks % 10_000_000) * 100) as u32,
        ));
        assert_eq!(state.modified_time().ok(), expected, "raw FILETIME={ticks}");
    }
}

#[test]
fn native_write_time_projection_matches_actual_file_metadata() {
    use std::fs::{File, FileTimes};
    use std::time::{Duration, SystemTime};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("timestamp.bin");
    std::fs::write(&path, b"original timestamp content").unwrap();
    for modified in [
        SystemTime::UNIX_EPOCH - Duration::from_nanos(100),
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH + Duration::from_nanos(100),
        SystemTime::UNIX_EPOCH + Duration::from_nanos(1_234_567_800),
        SystemTime::now(),
    ] {
        let file = File::options().read(true).write(true).open(&path).unwrap();
        file.set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        drop(file);
        let original = File::open(&path).unwrap();
        let state = WindowsFileState::capture(&original).unwrap();
        assert_eq!(
            state.modified_time().unwrap(),
            original.metadata().unwrap().modified().unwrap()
        );
    }
}
