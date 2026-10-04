//! 独立的重叠读管道：缓冲与 OVERLAPPED 地址跨 pending I/O 恒定。

use std::io;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
    GENERIC_WRITE, GetLastError,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_INBOUND, ReadFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, WaitForSingleObject,
};

use super::super::ChildError;
use super::owned_handle::OwnedHandle;
use super::pipe_security::PipeSecurity;

const READ_BYTES: usize = 4096;

/// 本任务一条管道及唯一 pending 读取。来源：Win32 overlapped named pipe、CancelIoEx 生命周期。
pub(super) struct OverlappedPipe {
    read: OwnedHandle,
    event: OwnedHandle,
    operation: Box<OVERLAPPED>,
    bytes: Box<[u8; READ_BYTES]>,
    pending: bool,
    eof: bool,
}

impl OverlappedPipe {
    /// 建立仅本机可连的随机命名管道。参数：name 是随机管道名，security 是显式 DACL。返回：父读端与子写端或错误。
    pub(super) fn create(
        name: &[u16],
        security: &PipeSecurity,
    ) -> Result<(Self, OwnedHandle), ChildError> {
        let read = OwnedHandle::from_raw(
            unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    READ_BYTES as u32,
                    READ_BYTES as u32,
                    0,
                    &security.attributes(false),
                )
            },
            "CreateNamedPipeW",
        )?;
        // CreateFileW 在服务器实例已存在后连接；只将写端列入 HANDLE_LIST。
        let mut writer_attributes = security.attributes(true);
        writer_attributes.lpSecurityDescriptor = null_mut();
        let writer = OwnedHandle::from_raw(
            unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &writer_attributes,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            },
            "CreateFileW(named pipe writer)",
        )?;
        let event = OwnedHandle::from_raw(
            unsafe { CreateEventW(null(), 1, 0, null()) },
            "CreateEventW(pipe operation)",
        )?;
        let mut operation = Box::new(OVERLAPPED::default());
        operation.hEvent = event.as_raw();
        // 客户端先连上时通常返回 ERROR_PIPE_CONNECTED；意外 pending 时，
        // 稳定的 Box 与 event 均保留到 GetOverlappedResult 确认内核完成。
        let result = unsafe { ConnectNamedPipe(read.as_raw(), operation.as_mut()) };
        if result == 0 {
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => {}
                ERROR_IO_PENDING => {
                    let mut ignored = 0u32;
                    let completed = loop {
                        let completed = unsafe {
                            GetOverlappedResult(read.as_raw(), operation.as_ref(), &mut ignored, 1)
                        };
                        if completed == 0 && unsafe { GetLastError() } == ERROR_IO_INCOMPLETE {
                            unsafe { WaitForSingleObject(event.as_raw(), INFINITE) };
                            continue;
                        }
                        break completed;
                    };
                    if completed == 0 {
                        return Err(last("GetOverlappedResult(pipe connect)"));
                    }
                }
                _ => return Err(last("ConnectNamedPipe")),
            }
        }
        Ok((
            Self {
                read,
                event,
                operation,
                bytes: Box::new([0; READ_BYTES]),
                pending: false,
                eof: false,
            },
            writer,
        ))
    }

    /// 非阻塞收取一次最多 4096 字节。参数：无。返回：数据切片、暂无数据/EOF 或 Win32 错误。
    pub(super) fn read_next(&mut self) -> Result<Option<&[u8]>, ChildError> {
        if self.eof {
            return Ok(None);
        }
        if self.pending {
            return self.finish_read(false, false);
        }
        if unsafe { ResetEvent(self.event.as_raw()) } == 0 {
            return Err(last("ResetEvent(pipe read)"));
        }
        *self.operation = OVERLAPPED::default();
        self.operation.hEvent = self.event.as_raw();
        let started = unsafe {
            ReadFile(
                self.read.as_raw(),
                self.bytes.as_mut_ptr(),
                READ_BYTES as u32,
                null_mut(),
                self.operation.as_mut(),
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

    /// 取消并确认唯一 pending 读完成。参数：无。返回：安全清理成功或 Win32 错误。
    pub(super) fn cancel_pending(&mut self) -> Result<(), ChildError> {
        if !self.pending {
            return Ok(());
        }
        let cancelled = unsafe { CancelIoEx(self.read.as_raw(), self.operation.as_ref()) };
        if cancelled == 0 && unsafe { GetLastError() } != ERROR_NOT_FOUND {
            // 即使取消请求失败，仍必须等待完成才能释放缓冲和 OVERLAPPED。
            let failure = last("CancelIoEx(pipe read)");
            let _ = self.finish_read(true, true);
            return Err(failure);
        }
        self.finish_read(true, true).map(|_| ())
    }

    fn finish_read(
        &mut self,
        wait: bool,
        owner_cancelled: bool,
    ) -> Result<Option<&[u8]>, ChildError> {
        let mut transferred = 0u32;
        let result = loop {
            let result = unsafe {
                GetOverlappedResult(
                    self.read.as_raw(),
                    self.operation.as_ref(),
                    &mut transferred,
                    i32::from(wait),
                )
            };
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
            if self.pending {
                unsafe { WaitForSingleObject(self.event.as_raw(), INFINITE) };
                let _ = self.finish_read(true, true);
            }
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
