//! Windows 目录身份的纯值资格回归；来源：windows_native_scan_root::alignment。
//! 真实目录不比较 EOF 与聚合树大小，因此记录 Unverified；此测试不代替 Windows 原生扫描。
use super::git_indexed_directory::windows_identity;
use diskgraph_core::{BusinessError, WindowsFileObservation, WindowsTreeAlignment};

fn captured_directory() -> WindowsFileObservation {
    WindowsFileObservation {
        volume: u64::MAX,
        file_id: [0xa5; 16],
        length: 0,
        creation_time: 133700000000000001,
        last_write_time: 133700000000000002,
        change_time: 133700000000000003,
        attributes: 0x10,
        directory: true,
        delete_pending: false,
        capture_started_unix_ms: 1,
        capture_finished_unix_ms: 2,
        tree_alignment: WindowsTreeAlignment::Unverified,
    }
}

#[test]
fn captured_windows_directory_identity_does_not_require_regular_file_size_alignment() {
    let observed = captured_directory();
    observed.validate().unwrap();
    assert!(
        observed.legacy_identity().is_none(),
        "full 128-bit identity must not be truncated"
    );
    assert_eq!(
        windows_identity(&observed).unwrap(),
        (observed.volume, observed.file_id, observed.creation_time)
    );
}

#[test]
fn windows_identity_rejects_non_directory_pending_delete_and_invalid_capture() {
    let mut non_directory = captured_directory();
    non_directory.directory = false;
    non_directory.attributes = 0;
    let mut deleted = captured_directory();
    deleted.delete_pending = true;
    let mut invalid = captured_directory();
    invalid.capture_started_unix_ms = invalid.capture_finished_unix_ms + 1;
    for observed in [non_directory, deleted, invalid] {
        assert_eq!(windows_identity(&observed), Err(BusinessError::Unsupported));
    }
}

#[test]
fn windows_directory_identity_refuses_reparse_and_materialization_attributes() {
    for flag in [0x400, 0x1000, 0x40000, 0x400000] {
        let mut observed = captured_directory();
        observed.attributes |= flag;
        assert_eq!(windows_identity(&observed), Err(BusinessError::Unsupported));
    }
}
