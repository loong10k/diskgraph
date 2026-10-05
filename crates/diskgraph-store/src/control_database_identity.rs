use rusqlite::Connection;

/// SQLite 实际已打开 main 文件的 Windows 完整原生身份，不表示路径或服务器逻辑身份。
/// 来源：SQLite WIN32_GET_HANDLE 与 Microsoft FILE_ID_INFO；Unix 尚无已验证的公开等价能力。
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ControlDatabaseIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

impl ControlDatabaseIdentity {
    /// 从 SQLite 自己已打开的句柄捕获身份，只借用，不关闭、复制或重新打开数据库文件。
    /// 参数：connection 为存活的独占控制连接；返回：合格默认 Windows VFS 的身份，未知能力为 None。
    #[cfg(windows)]
    pub(crate) fn capture(connection: &Connection) -> Option<Self> {
        use std::ffi::{CStr, c_void};
        let mut vfs: *mut rusqlite::ffi::sqlite3_vfs = std::ptr::null_mut();
        // 安全性：connection 存活；FILE_CONTROL 输出遵循公开 sqlite3_vfs** ABI。
        let vfs_result = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_VFS_POINTER,
                (&mut vfs as *mut *mut rusqlite::ffi::sqlite3_vfs).cast(),
            )
        };
        if vfs_result != rusqlite::ffi::SQLITE_OK || vfs.is_null() {
            return None;
        }
        // 安全性：SQLite 返回连接生命周期内有效的公开 VFS 结构及零结尾名称。
        let name = unsafe { (*vfs).zName };
        if name.is_null() {
            return None;
        }
        let name = unsafe { CStr::from_ptr(name) }.to_bytes();
        if !matches!(name, b"win32" | b"win32-longpath") {
            return None;
        }
        let mut handle: *mut c_void = std::ptr::null_mut();
        // 安全性：公开 WIN32_GET_HANDLE 只输出借用句柄，不访问私有 winFile 布局。
        let result = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_WIN32_GET_HANDLE,
                (&mut handle as *mut *mut c_void).cast(),
            )
        };
        if result != rusqlite::ffi::SQLITE_OK || handle.is_null() || handle as isize == -1 {
            return None;
        }
        let mut identity = Self {
            volume_serial: 0,
            file_id: [0; 16],
        };
        // 安全性：本 repr(C) 对象正是 FileIdInfo 的 u64+128bit 输出；原 SQLite 句柄仍存活。
        let ok = unsafe {
            get_file_information_by_handle_ex(
                handle,
                18,
                (&mut identity as *mut Self).cast(),
                std::mem::size_of::<Self>() as u32,
            )
        };
        (ok != 0).then_some(identity)
    }

    /// 保留没有可靠公开原生对象身份的平台的未知能力，不猜路径、UUID 或私有 fd。
    /// 参数：connection 为控制连接；返回：None，调用者仍须原实时 SQL 授权。
    #[cfg(not(windows))]
    pub(crate) fn capture(_connection: &Connection) -> Option<Self> {
        None
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetFileInformationByHandleEx"]
    fn get_file_information_by_handle_ex(
        handle: *mut std::ffi::c_void,
        class: i32,
        info: *mut std::ffi::c_void,
        size: u32,
    ) -> i32;
}
