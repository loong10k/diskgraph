//! Windows 正式 SQLite 文件控制与 FileIdInfo 的原生资格；不依赖尚缺的 watch API。
use super::withdrawal_test_fixture::WithdrawalFixture;
use crate::ControlStore;
use std::ffi::c_void;

/// Win32 FILE_ID_INFO 的公开 C ABI 测试投影；来源：Microsoft winbase.h。
#[repr(C)]
#[derive(Debug, Eq, PartialEq)]
struct OpenedMainIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetFileInformationByHandleEx"]
    fn get_file_information_by_handle_ex(
        handle: *mut c_void,
        class: i32,
        info: *mut c_void,
        size: u32,
    ) -> i32;
}

impl OpenedMainIdentity {
    fn capture(store: &ControlStore) -> Self {
        let mut handle: *mut c_void = std::ptr::null_mut();
        // 安全性：独占借用有效 Connection；官方 opcode 只返回借用 HANDLE，不关闭或更换它。
        let result = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                store.connection.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_WIN32_GET_HANDLE,
                (&mut handle as *mut *mut c_void).cast(),
            )
        };
        assert_eq!(
            result,
            rusqlite::ffi::SQLITE_OK,
            "actual main VFS must support official WIN32_GET_HANDLE"
        );
        assert!(
            !handle.is_null() && handle as isize != -1,
            "actual borrowed HANDLE qualification"
        );
        let mut identity = Self {
            volume_serial: 0,
            file_id: [0; 16],
        };
        // 安全性：FileIdInfo=18、repr(C) 输出长度与 Win32 FILE_ID_INFO 一致；原 HANDLE 保持打开。
        let ok = unsafe {
            get_file_information_by_handle_ex(
                handle,
                18,
                (&mut identity as *mut Self).cast(),
                std::mem::size_of::<Self>() as u32,
            )
        };
        assert_ne!(
            ok,
            0,
            "FileIdInfo qualification: {}",
            std::io::Error::last_os_error()
        );
        identity
    }
}

#[test]
fn official_main_handle_identifies_two_live_connections_without_reopening_path() {
    let f = WithdrawalFixture::new();
    let other = f.second();
    assert_eq!(
        OpenedMainIdentity::capture(&f.store),
        OpenedMainIdentity::capture(&other)
    );
    assert_eq!(
        f.store
            .live_permission(
                &f.principal,
                &diskgraph_core::Permission::MetadataRead,
                &f.scope
            )
            .unwrap(),
        Some(true)
    );
}

#[test]
fn same_server_sqlite_backup_is_a_different_actual_open_file() {
    let mut f = WithdrawalFixture::new();
    let mut backup = f.backup();
    assert_eq!(
        f.store.ensure_server().unwrap(),
        backup.ensure_server().unwrap()
    );
    assert_ne!(
        OpenedMainIdentity::capture(&f.store),
        OpenedMainIdentity::capture(&backup)
    );
}

#[test]
fn replacement_at_identical_path_has_a_new_actual_main_identity() {
    let f = WithdrawalFixture::new();
    let before = OpenedMainIdentity::capture(&f.store);
    let WithdrawalFixture {
        store,
        path,
        directory,
        ..
    } = f;
    drop(store);
    // Windows SQLite 可能禁止打开时 rename；关闭后真实替换，不伪造成功的路径竞争。
    std::fs::rename(&path, directory.path().join("original.sqlite")).unwrap();
    let replacement = ControlStore::open(&path).unwrap();
    assert_ne!(before, OpenedMainIdentity::capture(&replacement));
}
