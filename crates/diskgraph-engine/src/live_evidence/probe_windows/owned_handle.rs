//! Win32 HANDLE 的唯一所有权，避免异常路径泄漏本任务资源。

use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

use super::super::probe_failure::ProbeFailure;

/// 单一 Win32 句柄所有者。来源：Win32 CloseHandle 与 HANDLE 所有权契约。
pub(super) struct OwnedHandle(HANDLE);

impl OwnedHandle {
    /// 接管原始句柄。参数：raw 是 API 返回句柄，context 是错误位置。返回：唯一所有权或 Win32 错误。
    pub(super) fn from_raw(raw: HANDLE, context: &'static str) -> Result<Self, ProbeFailure> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(ProbeFailure::io(context, io::Error::last_os_error()));
        }
        Ok(Self(raw))
    }

    /// 借用原始句柄。参数：无。返回：仍由本对象持有的 HANDLE。
    pub(super) fn as_raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // 句柄值非空且非 INVALID；CloseHandle 仅作用于本对象所有权。
        unsafe { CloseHandle(self.0) };
    }
}
