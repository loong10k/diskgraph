#[cfg(windows)]
fn open_windows_child(
    parent: &File,
    name: &std::ffi::OsStr,
    create: bool,
    directory: bool,
    write: bool,
    share_mode: u32,
) -> Result<File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_NO_RECALL,
        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    };
    use windows_sys::Win32::Foundation::{
        CloseHandle, INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let mut wide: Vec<u16> = name.encode_wide().take(32768).collect();
    let length =
        u16::try_from(wide.len() * 2).map_err(|_| "unsupported private Git component length")?;
    if wide.is_empty() || wide.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92)) {
        return Err("invalid private Git component".into());
    }
    let unicode = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &unicode,
        Attributes: OBJ_DONT_REPARSE,
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_READ_ATTRIBUTES
                | SYNCHRONIZE
                | if write {
                    FILE_READ_DATA | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES
                } else {
                    0
                },
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            share_mode,
            if create { FILE_CREATE } else { FILE_OPEN },
            FILE_SYNCHRONOUS_IO_NONALERT
                | if directory {
                    // 新目录只能 FILE_CREATE；不会跟随既有目标，DIRFILE 不与 reparse/no-recall 混用。
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_OPEN_REPARSE_POINT | FILE_OPEN_NO_RECALL | FILE_NON_DIRECTORY_FILE
                },
            std::ptr::null(),
            0,
        )
    };
    if status != 0 || handle.is_null() || handle == INVALID_HANDLE_VALUE {
        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
        }
        return Err(format!(
            "private Git native create/open: {}",
            std::io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) as i32 })
        ));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}
