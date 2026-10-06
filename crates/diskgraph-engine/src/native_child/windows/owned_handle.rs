//! Win32 HANDLE 的唯一所有权，避免异常路径泄漏本任务资源。

use std::io;
use std::os::windows::io::{
    AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle as StdOwnedHandle,
};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};

use super::super::ChildError;

/// 单一 Win32 句柄所有者。来源：Win32 CloseHandle 与 HANDLE 所有权契约。
pub(crate) struct OwnedHandle(StdOwnedHandle);

impl OwnedHandle {
    /// 接管原始句柄。参数：raw 是 API 返回句柄，context 是错误位置。返回：唯一所有权或 Win32 错误。
    pub(crate) fn from_raw(raw: HANDLE, context: &'static str) -> Result<Self, ChildError> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(ChildError::io(context, io::Error::last_os_error()));
        }
        // 唯一有效Win32句柄转交标准库；其Send/Sync与CloseHandle合同无需重写。
        Ok(Self(unsafe { StdOwnedHandle::from_raw_handle(raw) }))
    }

    /// 安全借用原句柄。参数：无；返回：生命周期由本唯一所有者约束的句柄。
    pub(crate) fn as_handle(&self) -> BorrowedHandle<'_> {
        self.0.as_handle()
    }

    /// 借用原始句柄。参数：无。返回：仍由本对象持有的 HANDLE。
    pub(crate) fn as_raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
}
