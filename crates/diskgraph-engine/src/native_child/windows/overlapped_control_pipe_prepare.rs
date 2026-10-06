//! 原控制管道的外部 owner 准备；首次 Connect 在原槽安装后提交。
use super::super::{
    owned_handle::OwnedHandle, pipe_security::PipeSecurity,
    windows_overlapped_operation::WindowsOverlappedOperation,
};
use super::{OverlappedControlPipe, os_error};
use crate::native_child::{ChildError, ChildInputMode, ChildSpawnError, ControlWriteStatus};
use std::ptr::{null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GENERIC_READ, GetLastError,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_OUTBOUND,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::CreateEventW;

impl OverlappedControlPipe {
    /// 准备原 NUL 或显式控制 stdin。参数：mode 为输入模式，nonce 为本次随机名称，security 为原 DACL，owner 是原外槽，checkpoint 借用原期限。返回：唯一可继承子读端或原错误，控制 owner 保留在外槽。
    pub(in crate::native_child::windows) fn prepare_input_into<E>(
        mode: ChildInputMode,
        nonce: &uuid::Uuid,
        security: &PipeSecurity,
        owner: &mut Option<Self>,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<OwnedHandle, ChildSpawnError<E>> {
        if owner.is_some() {
            return Err(ChildError::Unsupported("control owner slot already occupied").into());
        }
        if mode == ChildInputMode::Null {
            // 保留原 NUL 句柄的属性、访问标志、错误文本和启动阶段，不新增检查点。
            let mut inheritable = security.attributes(true);
            inheritable.lpSecurityDescriptor = null_mut();
            let nul: Vec<u16> = r"\\.\NUL"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let stdin = OwnedHandle::from_raw(
                unsafe {
                    CreateFileW(
                        nul.as_ptr(),
                        GENERIC_READ,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        &inheritable,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        null_mut(),
                    )
                },
                "CreateFileW(NUL stdin)",
            )?;
            return Ok(stdin);
        }
        let name: Vec<u16> = format!(r"\\.\pipe\diskgraph-control-{nonce}-in")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        Self::create_into(&name, security, owner, checkpoint)
    }

    fn create_into<E>(
        name: &[u16],
        security: &PipeSecurity,
        owner: &mut Option<Self>,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<OwnedHandle, ChildSpawnError<E>> {
        let write = OwnedHandle::from_raw(
            unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    ControlWriteStatus::MAX_CHUNK_BYTES as u32,
                    ControlWriteStatus::MAX_CHUNK_BYTES as u32,
                    0,
                    &security.attributes(false),
                )
            },
            "CreateNamedPipeW(control input)",
        )?;
        let mut attributes = security.attributes(true);
        attributes.lpSecurityDescriptor = null_mut();
        let reader = OwnedHandle::from_raw(
            unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &attributes,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    null_mut(),
                )
            },
            "CreateFileW(control stdin)",
        )?;
        let event = OwnedHandle::from_raw(
            unsafe { CreateEventW(null(), 1, 0, null()) },
            "CreateEventW(control input)",
        )?;
        let operation = WindowsOverlappedOperation::new(event.as_raw());
        *owner = Some(Self {
            write: Some(write),
            event,
            operation,
            bytes: Box::new([0; ControlWriteStatus::MAX_CHUNK_BYTES]),
            length: 0,
            pending: false,
            connecting: true,
            closing: false,
            cancel_requested: false,
            cancel_accepted: false,
            #[cfg(test)]
            completion_error: None,
        });
        let channel = owner.as_mut().expect("original external control owner");
        let connected = unsafe { ConnectNamedPipe(channel.handle()?, channel.operation.as_ptr()) };
        let error = if connected == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        if connected != 0 || error == ERROR_PIPE_CONNECTED {
            channel.connecting = false;
            return Ok(reader);
        }
        if error != ERROR_IO_PENDING {
            channel.pending = channel.operation.pending();
            return Err(os_error("ConnectNamedPipe(control input)", error).into());
        }
        channel.pending = true;
        loop {
            if let Err(primary) = checkpoint() {
                drop(reader);
                return Err(ChildSpawnError::checkpoint(primary));
            }
            match channel.finish_io(false) {
                Ok(ControlWriteStatus::Pending) => std::thread::sleep(Duration::from_millis(1)),
                Ok(ControlWriteStatus::Written(_)) => return Ok(reader),
                Ok(ControlWriteStatus::Closed) => {
                    return Err(ChildError::Unsupported("control connect closed").into());
                }
                Err(primary) => {
                    drop(reader);
                    return Err(primary.into());
                }
            }
        }
    }
}
