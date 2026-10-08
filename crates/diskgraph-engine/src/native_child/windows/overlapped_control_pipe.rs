//! 独占控制输入写管道；业务轮询不等待，安全清理保留内核借用的稳定内存直到完成。

use super::cleanup_progress::CleanupProgress;
use std::io;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, GetLastError,
    HANDLE,
};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
#[cfg(test)]
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};
use windows_sys::Win32::System::Threading::ResetEvent;

use super::super::{ChildError, ControlWriteStatus};
use super::owned_handle::OwnedHandle;
use super::windows_overlapped_operation::WindowsOverlappedOperation;

#[path = "overlapped_control_pipe_prepare.rs"]
mod overlapped_control_pipe_prepare;

/// 父进程唯一控制写端与一块稳定内存。来源：Win32 outbound named pipe、OVERLAPPED 和 CancelIoEx 完成契约。
pub(super) struct OverlappedControlPipe {
    write: Option<OwnedHandle>,
    event: OwnedHandle,
    operation: WindowsOverlappedOperation,
    bytes: Box<[u8; ControlWriteStatus::MAX_CHUNK_BYTES]>,
    length: usize,
    pending: bool,
    connecting: bool,
    closing: bool,
    cancel_requested: bool,
    cancel_accepted: bool,
    #[cfg(test)]
    completion_error: Option<u32>,
}

impl OverlappedControlPipe {
    /// 借用原 pending 控制操作的事件。参数：无；返回：原 owner 持有的完成事件或空。
    /// 唤醒不替代 poll_write 对实际字节与取消完成的观察。
    pub(super) fn pending_event(&self) -> Option<HANDLE> {
        self.pending.then(|| self.event.as_raw())
    }

    /// 写入一个自有数据块，禁止覆盖未完成块。参数：source 非空且最多 4096 字节。返回：真实内核完成字节数、Pending 或原 I/O 错误；不证明 worker 已读取。
    pub(super) fn start_write(&mut self, source: &[u8]) -> Result<ControlWriteStatus, ChildError> {
        if self.connecting || self.closing || self.write.is_none() {
            return Err(ChildError::Unsupported(
                "control input already closing or closed",
            ));
        }
        if self.pending {
            return Err(ChildError::Unsupported("control write already pending"));
        }
        if source.is_empty() || source.len() > self.bytes.len() {
            return Err(ChildError::Unsupported(
                "control write requires 1..=4096 bytes",
            ));
        }
        if unsafe { ResetEvent(self.event.as_raw()) } == 0 {
            return Err(last("ResetEvent(control write)"));
        }
        self.operation.reset(self.event.as_raw());
        self.bytes[..source.len()].copy_from_slice(source);
        self.length = source.len();
        self.cancel_requested = false;
        self.cancel_accepted = false;
        self.pending = true;
        let started = unsafe {
            WriteFile(
                self.handle()?,
                self.bytes.as_ptr(),
                self.length as u32,
                null_mut(),
                self.operation.as_ptr(),
            )
        };
        let error = if started == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        if started != 0 {
            return self.finish_io(false);
        }
        if error == ERROR_IO_PENDING {
            Ok(ControlWriteStatus::Pending)
        } else {
            self.pending = false;
            self.length = 0;
            Err(os_error("WriteFile(control input)", error))
        }
    }

    /// 只查询同一个未完成写入。参数：无。返回：Pending、一次真实 Written 或 Closed；空闲轮询被拒绝。
    pub(super) fn poll_write(&mut self) -> Result<ControlWriteStatus, ChildError> {
        if self.pending {
            self.finish_io(false)
        } else if self.write.is_none() {
            Ok(ControlWriteStatus::Closed)
        } else {
            Err(ChildError::Unsupported("no pending control write"))
        }
    }

    /// 封闭后续写入并只请求一次取消。参数：无。返回：Closed 或仍持有内核借用内存的 Pending；正常完成竞争由随后 poll 返回 Written 一次。
    pub(super) fn request_close(&mut self) -> Result<ControlWriteStatus, ChildError> {
        self.closing = true;
        if !self.pending {
            self.write.take();
            return Ok(ControlWriteStatus::Closed);
        }
        if !self.cancel_requested {
            self.cancel_requested = true;
            let cancelled = unsafe { CancelIoEx(self.handle()?, self.operation.as_ptr()) };
            let error = if cancelled == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            self.cancel_accepted = cancelled != 0;
            if cancelled == 0 && error != ERROR_NOT_FOUND {
                return Err(os_error("CancelIoEx(control input)", error));
            }
            // NOT_FOUND 只表示未找到可取消请求，不能释放内存或当成完成。
        }
        Ok(ControlWriteStatus::Pending)
    }

    /// 参数：deadline为原处置绝对期限；返回：一次非阻塞观察，Pending/Err保留原稳定内存与未完成写责任。
    /// 不使用bWait=TRUE；Cancel成功或NOT_FOUND均不代表完成，最后期限检查前不关闭写句柄。
    pub(super) fn poll_cleanup(
        &mut self,
        deadline: Instant,
    ) -> Result<CleanupProgress, ChildError> {
        if self.write.is_none() && !self.pending {
            return Ok(CleanupProgress::Complete);
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        self.closing = true;
        if self.pending && !self.cancel_accepted {
            let cancelled = unsafe { CancelIoEx(self.handle()?, self.operation.as_ptr()) };
            let error = if cancelled == 0 {
                unsafe { GetLastError() }
            } else {
                0
            };
            if cancelled == 0 && error != ERROR_NOT_FOUND {
                return Err(os_error("CancelIoEx(control poll cleanup)", error));
            }
            self.cancel_requested = true;
            self.cancel_accepted = cancelled != 0;
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        if self.pending {
            self.finish_io_with_release(false, false)?;
        }
        if self.pending || Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        self.write.take();
        Ok(CleanupProgress::Complete)
    }

    /// 安全收取取消后的实际完成。参数：无。返回：清理成功或原取消／完成错误；本操作可超出协作期限，业务循环只能使用非阻塞方法。
    pub(super) fn cleanup(&mut self) -> Result<(), ChildError> {
        let requested = self.request_close().map(|_| ());
        let mut completion_failure = None;
        while self.pending {
            if let Err(error) = self.finish_io(true)
                && completion_failure.is_none()
            {
                // 异常查询的重复失败不累积错误链；保留第一项真实完成错误。
                completion_failure = Some(error);
            }
            if self.pending {
                // bWait=TRUE 正常会等到完成；异常查询仍不能释放活跃内存，也不紧循环。
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        self.write.take();
        let completion = completion_failure.map_or(Ok(()), Err);
        match requested {
            Err(primary) => Err(primary.with_cleanup(completion)),
            Ok(()) => completion,
        }
    }

    /// 借用真实 pending 写所使用的句柄和稳定内存。参数：无。返回：仅测试可见的原始地址，不复制或改变 operation。
    #[cfg(test)]
    pub(super) fn io_witness(&self) -> Result<(HANDLE, *const OVERLAPPED, *const u8), ChildError> {
        if !self.pending {
            return Err(ChildError::Unsupported(
                "no actual pending control operation",
            ));
        }
        Ok((self.handle()?, self.operation.as_ptr(), self.bytes.as_ptr()))
    }

    /// 借用父端句柄供真实继承断言。参数：无。返回：父写端和父 event，不转移所有权。
    #[cfg(test)]
    pub(super) fn handles(&self) -> Result<(HANDLE, HANDLE), ChildError> {
        Ok((self.handle()?, self.event.as_raw()))
    }
    /// 参数：无；返回：最后实际完成查询的原生错误码，尚未记录时为 None。
    /// 仅记录实际GetOverlappedResult终态，供接管线程证明取消完成而非线程退出。
    #[cfg(test)]
    pub(super) fn completion_error_for_test(&self) -> Option<u32> {
        self.completion_error
    }

    fn handle(&self) -> Result<HANDLE, ChildError> {
        self.write
            .as_ref()
            .map(OwnedHandle::as_raw)
            .ok_or(ChildError::Unsupported(
                "control write handle already closed",
            ))
    }

    fn finish_io(&mut self, wait: bool) -> Result<ControlWriteStatus, ChildError> {
        self.finish_io_with_release(wait, true)
    }

    // 有限处置只查询真实完成，原兼容调用保持原closing时立即释放的语义。
    fn finish_io_with_release(
        &mut self,
        wait: bool,
        release: bool,
    ) -> Result<ControlWriteStatus, ChildError> {
        let mut transferred = 0;
        let completed = unsafe {
            GetOverlappedResult(
                self.handle()?,
                self.operation.as_ptr(),
                &mut transferred,
                i32::from(wait),
            )
        };
        let error = if completed == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        if completed == 0 && error == ERROR_IO_INCOMPLETE {
            return Ok(ControlWriteStatus::Pending);
        }
        // 等同 Win32 HasOverlappedIoCompleted：查询失败也不能释放仍标记 STATUS_PENDING 的内存。
        if completed == 0 && self.operation.pending() {
            return Err(os_error("GetOverlappedResult(control still active)", error));
        }
        let connecting = self.connecting;
        #[cfg(test)]
        {
            self.completion_error = (completed == 0).then_some(error);
        }
        self.connecting = false;
        self.pending = false;
        let length = std::mem::take(&mut self.length);
        if self.closing && release {
            self.write.take();
        }
        if completed == 0 {
            if error == ERROR_OPERATION_ABORTED && self.closing && self.cancel_accepted {
                return Ok(ControlWriteStatus::Closed);
            }
            return Err(os_error("GetOverlappedResult(control input)", error));
        }
        if connecting {
            // ConnectNamedPipe 的 transferred 未定义，绝不用于写入游标。
            return Ok(ControlWriteStatus::Written(0));
        }
        if transferred == 0 {
            return Err(ChildError::io(
                "GetOverlappedResult(control input)",
                io::Error::new(
                    io::ErrorKind::WriteZero,
                    "nonempty control write completed with zero bytes",
                ),
            ));
        }
        if transferred as usize > length {
            return Err(ChildError::Unsupported(
                "control completion exceeds owned chunk",
            ));
        }
        Ok(ControlWriteStatus::Written(transferred as usize))
    }
}

impl Drop for OverlappedControlPipe {
    fn drop(&mut self) {
        // 仅安全退场可等待；原业务 poll/connect 不使用 INFINITE 或 bWait=TRUE。
        let _ = self.cleanup();
    }
}

fn os_error(context: &'static str, code: u32) -> ChildError {
    ChildError::io(context, io::Error::from_raw_os_error(code as i32))
}

fn last(context: &'static str) -> ChildError {
    ChildError::io(context, io::Error::last_os_error())
}
