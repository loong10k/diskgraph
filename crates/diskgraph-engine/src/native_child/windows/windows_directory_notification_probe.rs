//! 原生通知时机实验：不替代删除证明，不接通产品清理或释放容量。
use super::owned_handle::OwnedHandle;
use super::windows_overlapped_operation::WindowsOverlappedOperation;
use std::cell::UnsafeCell;
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{
    ERROR_IO_INCOMPLETE, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, GetLastError,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, ReadDirectoryChangesExW,
    ReadDirectoryNotifyExtendedInformation,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};
use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent};

/// 唯一测试通知owner，稳定存储保留至实际I/O完成。来源：Win32目录通知合同，无Java对应。
pub(super) struct WindowsDirectoryNotificationProbe {
    directory: File,
    event: OwnedHandle,
    operation: WindowsOverlappedOperation,
    bytes: Box<UnsafeCell<[u64; 8192]>>,
    pending: bool,
}

impl WindowsDirectoryNotificationProbe {
    /// 参数：directory为已打开的异步原父目录；返回：已提交的原通知owner或原生错误。
    pub(super) fn new(directory: File) -> io::Result<Self> {
        let event = OwnedHandle::from_raw(
            unsafe { CreateEventW(null_mut(), 1, 0, null_mut()) },
            "CreateEventW(directory notification probe)",
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        let operation = WindowsOverlappedOperation::new(event.as_raw());
        let mut owner = Self {
            directory,
            event,
            operation,
            bytes: Box::new(UnsafeCell::new([0; 8192])),
            pending: false,
        };
        owner.arm()?;
        Ok(owner)
    }

    fn arm(&mut self) -> io::Result<()> {
        assert!(!self.pending, "must complete original notification first");
        if unsafe { ResetEvent(self.event.as_raw()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.operation.reset(self.event.as_raw());
        // 不建立指向pending内核可变存储的Rust引用；Box地址不因owner移动改变。
        let result = unsafe {
            ReadDirectoryChangesExW(
                self.directory.as_raw_handle(),
                self.bytes.get().cast(),
                65536,
                0,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
                null_mut(),
                self.operation.as_ptr(),
                None,
                ReadDirectoryNotifyExtendedInformation,
            )
        };
        self.pending = result != 0 || self.operation.pending();
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// 参数：无；返回：本次已完成的原始通知或尚未完成。记录丢失/错误不能表示对象已移除。
    pub(super) fn poll(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut transferred = 0;
        let result = unsafe {
            GetOverlappedResult(
                self.directory.as_raw_handle(),
                self.operation.as_ptr(),
                &mut transferred,
                0,
            )
        };
        if result == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_INCOMPLETE {
                return Ok(None);
            }
            self.pending = self.operation.pending();
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        self.pending = false;
        if transferred == 0 || transferred > 65536 {
            return Err(io::Error::other("directory notification records lost"));
        }
        // 实际完成后才读取，复制本次有限记录再提交下一次原I/O。
        let bytes = unsafe {
            std::slice::from_raw_parts(self.bytes.get().cast::<u8>(), transferred as usize)
        }
        .to_vec();
        self.arm()?;
        Ok(Some(bytes))
    }

    /// 参数：无；返回：取消后的实际完成状态；未完成或错误保留原内存与所有句柄。
    pub(super) fn poll_cancel(&mut self) -> io::Result<bool> {
        if !self.pending {
            return Ok(true);
        }
        let result = unsafe { CancelIoEx(self.directory.as_raw_handle(), self.operation.as_ptr()) };
        if result == 0 && unsafe { GetLastError() } != ERROR_NOT_FOUND {
            return Err(io::Error::last_os_error());
        }
        let mut transferred = 0;
        let result = unsafe {
            GetOverlappedResult(
                self.directory.as_raw_handle(),
                self.operation.as_ptr(),
                &mut transferred,
                0,
            )
        };
        let error = if result == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        self.pending = result == 0 && self.operation.pending();
        if result != 0 || (!self.pending && error == ERROR_OPERATION_ABORTED) {
            Ok(true)
        } else if error == ERROR_IO_INCOMPLETE {
            Ok(false)
        } else {
            Err(io::Error::from_raw_os_error(error as i32))
        }
    }
}

impl Drop for WindowsDirectoryNotificationProbe {
    fn drop(&mut self) {
        // 实验失败也不能释放仍被内核引用的存储；此测试兜底不宣称有限退出。
        while self.pending {
            let _ = self.poll_cancel();
            if self.pending {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}
