//! 独立的重叠读管道：缓冲与 OVERLAPPED 地址跨 pending I/O 恒定。

use super::cleanup_progress::CleanupProgress;
use std::io;
use std::ptr::null_mut;
use std::time::Instant;
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, ERROR_PIPE_NOT_CONNECTED, GetLastError,
};
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};
use windows_sys::Win32::System::Threading::{INFINITE, ResetEvent, WaitForSingleObject};

use super::super::ChildError;
use super::owned_handle::OwnedHandle;
use super::windows_overlapped_operation::WindowsOverlappedOperation;
use super::windows_pipe_io_phase::WindowsPipeIoPhase;

#[path = "overlapped_pipe_prepare.rs"]
mod overlapped_pipe_prepare;

const READ_BYTES: usize = 4096;
#[cfg(test)]
thread_local! {
    static FAIL_QUERY_ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 本任务一条管道及唯一 pending 读取。来源：Win32 overlapped named pipe、CancelIoEx 生命周期。
pub(super) struct OverlappedPipe {
    read: OwnedHandle,
    event: OwnedHandle,
    operation: WindowsOverlappedOperation,
    bytes: Box<[u8; READ_BYTES]>,
    pending: bool,
    phase: WindowsPipeIoPhase,
    eof: bool,
    cleanup_started: bool,
    cleanup_cancel_requested: bool,
    cleanup_complete: bool,
}

impl OverlappedPipe {
    /// 非阻塞收取一次最多 4096 字节。参数：无。返回：数据切片、暂无数据/EOF 或 Win32 错误。
    pub(super) fn read_next(&mut self) -> Result<Option<&[u8]>, ChildError> {
        if self.eof {
            return Ok(None);
        }
        if self.cleanup_started {
            return Err(ChildError::Unsupported("pipe read cleanup already started"));
        }
        if self.phase == WindowsPipeIoPhase::Prepared {
            return Err(ChildError::Unsupported("pipe connection not submitted"));
        }
        if self.phase == WindowsPipeIoPhase::Connecting {
            self.finish_connect(false, false)?;
            if self.pending {
                return Ok(None);
            }
        }
        if self.pending {
            return self.finish_read(false, false);
        }
        if unsafe { ResetEvent(self.event.as_raw()) } == 0 {
            return Err(last("ResetEvent(pipe read)"));
        }
        self.operation.reset(self.event.as_raw());
        self.phase = WindowsPipeIoPhase::Reading;
        let started = unsafe {
            ReadFile(
                self.read.as_raw(),
                self.bytes.as_mut_ptr(),
                READ_BYTES as u32,
                null_mut(),
                self.operation.as_ptr(),
            )
        };
        if started != 0 {
            return self.finish_read(false, false);
        }
        let error = unsafe { GetLastError() };
        if error == ERROR_IO_PENDING {
            self.pending = true;
            Ok(None)
        } else if is_eof(error) {
            self.eof = true;
            Ok(None)
        } else {
            Err(last("ReadFile(overlapped pipe)"))
        }
    }

    /// 查询已确认的管道 EOF。参数：无。返回：确认 EOF 时为 true。
    pub(super) fn eof(&self) -> bool {
        self.eof
    }

    /// 参数：deadline为同一处置绝对期限；返回：一次非阻塞完成观察或原生错误，不释放活跃缓冲。
    /// Pending/Err保留原句柄与稳定地址；过期不请求取消、不将未完成读当作EOF。
    pub(super) fn poll_cleanup(
        &mut self,
        deadline: Instant,
    ) -> Result<CleanupProgress, ChildError> {
        if self.cleanup_complete {
            return Ok(CleanupProgress::Complete);
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        let was_connecting = matches!(
            self.phase,
            WindowsPipeIoPhase::Prepared | WindowsPipeIoPhase::Connecting
        );
        self.cleanup_started = true;
        if self.pending && !self.cleanup_cancel_requested {
            let cancelled = unsafe { CancelIoEx(self.read.as_raw(), self.operation.as_ptr()) };
            let error = if cancelled == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            if cancelled == 0 && error != ERROR_NOT_FOUND {
                return Err(ChildError::io(
                    "CancelIoEx(pipe poll cleanup)",
                    io::Error::from_raw_os_error(error as i32),
                ));
            }
            self.cleanup_cancel_requested = true;
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        if self.pending {
            if self.phase == WindowsPipeIoPhase::Connecting {
                self.finish_connect(false, true)?;
            } else {
                self.finish_read(false, true)?;
            }
        }
        if self.pending || Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        self.cleanup_complete = true;
        if !was_connecting {
            self.eof = true;
        }
        Ok(CleanupProgress::Complete)
    }

    /// 取消并确认唯一 pending 读完成。参数：无。返回：安全清理成功或 Win32 错误。
    pub(super) fn cancel_pending(&mut self) -> Result<(), ChildError> {
        if !self.pending {
            return Ok(());
        }
        let cancelled = unsafe { CancelIoEx(self.read.as_raw(), self.operation.as_ptr()) };
        if cancelled == 0 && unsafe { GetLastError() } != ERROR_NOT_FOUND {
            // 即使取消请求失败，仍必须等待完成才能释放缓冲和 OVERLAPPED。
            let failure = last("CancelIoEx(pipe read)");
            return self.finish_pending(Some(failure));
        }
        self.finish_pending(None)
    }

    fn finish_pending(&mut self, mut failure: Option<ChildError>) -> Result<(), ChildError> {
        while self.pending {
            let result = if self.phase == WindowsPipeIoPhase::Connecting {
                self.finish_connect(true, true)
            } else {
                self.finish_read(true, true).map(|_| ())
            };
            if let Err(error) = result
                && failure.is_none()
            {
                failure = Some(error);
            }
            if self.pending {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        failure.map_or(Ok(()), Err)
    }
    /// 参数：无；返回：原 pending 操作的 HANDLE、OVERLAPPED 和缓冲区地址；没有 pending 时拒绝。

    /// 只供真实pending读跨线程测试观察，不修改原内存或句柄。
    #[cfg(test)]
    pub(super) fn io_witness(
        &self,
    ) -> Result<
        (
            windows_sys::Win32::Foundation::HANDLE,
            *const windows_sys::Win32::System::IO::OVERLAPPED,
            *const u8,
        ),
        ChildError,
    > {
        if !self.pending {
            return Err(ChildError::Unsupported("no pending read witness"));
        }
        Ok((
            self.read.as_raw(),
            self.operation.as_ptr(),
            self.bytes.as_ptr(),
        ))
    }
    /// 参数：无；返回：当前操作是否仍处于连接阶段，不表示读取完成。

    /// 测试区分真实 Connect 与 Read；不修改内核操作。
    #[cfg(test)]
    pub(super) fn connecting_for_test(&self) -> bool {
        self.phase == WindowsPipeIoPhase::Connecting
    }
    /// 参数：无；返回：无；在当前测试线程设置一次查询故障，不取消原 pending 操作。

    /// 一次性注入查询失败，不声称内核拒权；实际pending仍由原OS操作建立。
    #[cfg(test)]
    pub(super) fn fail_next_query_for_test() {
        FAIL_QUERY_ONCE.with(|state| state.set(true));
    }

    fn query_result(&self, transferred: &mut u32, wait: bool) -> i32 {
        #[cfg(test)]
        if FAIL_QUERY_ONCE.with(|state| state.replace(false)) {
            unsafe {
                windows_sys::Win32::Foundation::SetLastError(
                    windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED,
                )
            };
            return 0;
        }
        unsafe {
            GetOverlappedResult(
                self.read.as_raw(),
                self.operation.as_ptr(),
                transferred,
                i32::from(wait),
            )
        }
    }

    fn finish_read(
        &mut self,
        wait: bool,
        owner_cancelled: bool,
    ) -> Result<Option<&[u8]>, ChildError> {
        let mut transferred = 0u32;
        let result = loop {
            let result = self.query_result(&mut transferred, wait);
            if result == 0 && wait && unsafe { GetLastError() } == ERROR_IO_INCOMPLETE {
                // 理论上 bWait=TRUE 不会返回未完成；保守等待，绝不释放活跃缓冲。
                unsafe { WaitForSingleObject(self.event.as_raw(), INFINITE) };
                continue;
            }
            break result;
        };
        if result == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_INCOMPLETE && !wait {
                self.pending = true;
                return Ok(None);
            }
            if self.operation.pending() {
                self.pending = true;
                return Err(ChildError::io(
                    "GetOverlappedResult(pipe still active)",
                    io::Error::from_raw_os_error(error as i32),
                ));
            }
            self.pending = false;
            if is_terminal_read(error, owner_cancelled) {
                self.eof = true;
                return Ok(None);
            }
            return Err(last("GetOverlappedResult(pipe read)"));
        }
        self.pending = false;
        if transferred == 0 {
            // Win32 ReadFile 成功且 0 字节也可能来自零长度 WriteFile，不能推断 EOF。
            Ok(None)
        } else {
            Ok(Some(&self.bytes[..transferred as usize]))
        }
    }
}

impl Drop for OverlappedPipe {
    fn drop(&mut self) {
        // 安全清理可超出协作期限。绝不在内核仍引用缓冲时释放 Box。
        if self.pending {
            let _ = self.cancel_pending();
        }
    }
}

fn is_eof(error: u32) -> bool {
    matches!(
        error,
        ERROR_BROKEN_PIPE | ERROR_HANDLE_EOF | ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED
    )
}

fn is_terminal_read(error: u32, owner_cancelled: bool) -> bool {
    is_eof(error) || (owner_cancelled && error == ERROR_OPERATION_ABORTED)
}

fn last(context: &'static str) -> ChildError {
    ChildError::io(context, io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::{is_eof, is_terminal_read};
    use windows_sys::Win32::Foundation::{
        ERROR_BROKEN_PIPE, ERROR_IO_INCOMPLETE, ERROR_OPERATION_ABORTED,
    };

    #[test]
    fn only_disconnection_codes_are_eof() {
        assert!(is_eof(ERROR_BROKEN_PIPE));
        assert!(!is_eof(ERROR_OPERATION_ABORTED));
        assert!(!is_eof(ERROR_IO_INCOMPLETE));
        assert!(!is_terminal_read(ERROR_OPERATION_ABORTED, false));
        assert!(is_terminal_read(ERROR_OPERATION_ABORTED, true));
    }
}
