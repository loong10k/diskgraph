//! 将稳定内存与原生句柄交给外部 owner 后，才允许首次提交连接。
use super::super::super::ChildError;
use super::super::windows_overlapped_operation::WindowsOverlappedOperation;
use super::super::windows_pipe_io_phase::WindowsPipeIoPhase;
use super::super::{owned_handle::OwnedHandle, pipe_security::PipeSecurity};
use super::{OverlappedPipe, READ_BYTES, last};
use std::{
    io,
    ptr::{null, null_mut},
};
use windows_sys::Win32::Foundation::{
    ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED,
    GENERIC_WRITE, GetLastError,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::CreateEventW;

impl OverlappedPipe {
    /// 将尚未提交 I/O 的管道安装到外部 owner。参数：唯一名称、DACL、空槽位；返回：原生错误。
    pub(in crate::native_child::windows) fn prepare_into(
        name: &[u16],
        security: &PipeSecurity,
        owner: &mut Option<Self>,
    ) -> Result<(), ChildError> {
        if owner.is_some() {
            return Err(ChildError::Unsupported(
                "prepared pipe owner already occupied",
            ));
        }
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
        let event = OwnedHandle::from_raw(
            unsafe { CreateEventW(null(), 1, 0, null()) },
            "CreateEventW(pipe operation)",
        )?;
        let operation = WindowsOverlappedOperation::new(event.as_raw());
        *owner = Some(Self {
            read,
            event,
            operation,
            bytes: Box::new([0; READ_BYTES]),
            pending: false,
            phase: WindowsPipeIoPhase::Prepared,
            eof: false,
            cleanup_started: false,
            cleanup_cancel_requested: false,
            cleanup_complete: false,
        });
        Ok(())
    }

    /// 打开仅用于子进程继承的写端。参数：已存在的唯一名称与 DACL；返回：写端所有权。
    pub(in crate::native_child::windows) fn open_writer(
        name: &[u16],
        security: &PipeSecurity,
    ) -> Result<OwnedHandle, ChildError> {
        let mut attributes = security.attributes(true);
        attributes.lpSecurityDescriptor = null_mut();
        OwnedHandle::from_raw(
            unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &attributes,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            },
            "CreateFileW(named pipe writer)",
        )
    }

    /// 在原外部 owner 上提交连接，不等待完成。参数：无；返回：提交错误，pending 仍归原 owner。
    pub(in crate::native_child::windows) fn start_connect(&mut self) -> Result<(), ChildError> {
        if self.pending || self.phase != WindowsPipeIoPhase::Prepared || self.cleanup_started {
            return Err(ChildError::Unsupported(
                "pipe connection already submitted or cleaning",
            ));
        }
        self.phase = WindowsPipeIoPhase::Connecting;
        let result = unsafe { ConnectNamedPipe(self.read.as_raw(), self.operation.as_ptr()) };
        if result != 0 {
            self.phase = WindowsPipeIoPhase::Ready;
            return Ok(());
        }
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => {
                self.phase = WindowsPipeIoPhase::Ready;
                Ok(())
            }
            ERROR_IO_PENDING => {
                self.pending = true;
                Ok(())
            }
            _ => {
                // 意外错误也不能丢失仍由内核引用的原始地址。
                self.pending = self.operation.pending();
                Err(last("ConnectNamedPipe"))
            }
        }
    }

    /// 参数：无；返回：原连接已真实完成时为 true；pending 地址仍留在原外槽。
    pub(in crate::native_child::windows) fn connect_ready(&mut self) -> Result<bool, ChildError> {
        if self.phase == WindowsPipeIoPhase::Connecting && self.pending {
            self.finish_connect(false, false)?;
        }
        Ok(self.phase == WindowsPipeIoPhase::Ready && !self.pending)
    }
    /// 参数：wait 指定是否同步等待，owner_cancelled 表示原 owner 已请求取消；返回：连接完成或原查询错误，未完成时保留 pending 责任，不标记读取 EOF。
    pub(super) fn finish_connect(
        &mut self,
        wait: bool,
        owner_cancelled: bool,
    ) -> Result<(), ChildError> {
        let mut ignored = 0;
        let result = self.query_result(&mut ignored, wait);
        if result == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_INCOMPLETE || self.operation.pending() {
                self.pending = true;
                if error == ERROR_IO_INCOMPLETE {
                    return Ok(());
                }
                return Err(ChildError::io(
                    "GetOverlappedResult(connect still active)",
                    io::Error::from_raw_os_error(error as i32),
                ));
            }
            self.pending = false;
            if owner_cancelled && error == ERROR_OPERATION_ABORTED {
                // 保留 Connecting，处置完成也不制造正常读取 EOF。
                return Ok(());
            }
            return Err(ChildError::io(
                "GetOverlappedResult(pipe connect)",
                io::Error::from_raw_os_error(error as i32),
            ));
        }
        self.pending = false;
        self.phase = WindowsPipeIoPhase::Ready;
        Ok(())
    }
}
