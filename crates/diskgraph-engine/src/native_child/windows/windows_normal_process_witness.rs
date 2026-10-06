//! 普通目标进程的原句柄身份见证；总Job计数不能替代特定后代存活证据。
use super::super::ChildError;
use super::owned_handle::OwnedHandle;
use std::io;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

/// 仅持查询/同步句柄的后代见证；来源：Win32进程创建时间、Job成员和真实wait。
pub(super) struct WindowsNormalProcessWitness {
    handle: OwnedHandle,
    identity: [u8; 12],
}
impl WindowsNormalProcessWitness {
    /// 参数：原出生句柄；返回固定PID+creation记录，不从名称或Job总数推断身份。
    pub(super) fn record(handle: HANDLE) -> Result<[u8; 12], ChildError> {
        let pid = unsafe { GetProcessId(handle) };
        if pid == 0 {
            return Err(last("GetProcessId(normal witness)"));
        }
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0
        {
            return Err(last("GetProcessTimes(normal witness)"));
        }
        let time = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        let mut record = [0; 12];
        record[..4].copy_from_slice(&pid.to_le_bytes());
        record[4..].copy_from_slice(&time.to_le_bytes());
        Ok(record)
    }
    /// 参数：原出生记录及原Job；返回绑定同一存活进程的独占只读见证，PID重用拒绝。
    pub(super) fn open(identity: [u8; 12], job: HANDLE) -> Result<Self, ChildError> {
        let pid = u32::from_le_bytes(identity[..4].try_into().unwrap());
        let handle = OwnedHandle::from_raw(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            },
            "OpenProcess(normal target)",
        )?;
        Self::verify_live(handle.as_raw(), &identity, job)?;
        Ok(Self { handle, identity })
    }
    /// 参数：原held句柄、记录与Job；返回真实同身份、Job成员且未退出，否则原错误。
    pub(super) fn verify_live(
        handle: HANDLE,
        identity: &[u8; 12],
        job: HANDLE,
    ) -> Result<(), ChildError> {
        if &Self::record(handle)? != identity {
            return Err(ChildError::Unsupported("normal target identity changed"));
        }
        let mut member = 0;
        if unsafe { IsProcessInJob(handle, job, &mut member) } == 0 {
            return Err(last("IsProcessInJob(normal target)"));
        }
        if member == 0 {
            return Err(ChildError::Unsupported(
                "normal target outside original Job",
            ));
        }
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_TIMEOUT => Ok(()),
            WAIT_OBJECT_0 => Err(ChildError::Unsupported(
                "normal target exited before release",
            )),
            _ => Err(last("WaitForSingleObject(normal target alive)")),
        }
    }
    /// 参数：原Job；返回同一后代当前真实存活状态的强验证。
    pub(super) fn alive(&self, job: HANDLE) -> Result<(), ChildError> {
        Self::verify_live(self.handle.as_raw(), &self.identity, job)
    }
    /// 参数：无；返回同一后代真实已wait且自然exit0，未完成不能当成功。
    pub(super) fn exited_zero(&self) -> Result<(), ChildError> {
        if Self::record(self.handle.as_raw())? != self.identity {
            return Err(ChildError::Unsupported(
                "normal target final identity changed",
            ));
        }
        match unsafe { WaitForSingleObject(self.handle.as_raw(), 0) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Err(ChildError::Unsupported("normal target not yet reaped")),
            _ => return Err(last("WaitForSingleObject(normal target exit)")),
        }
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.handle.as_raw(), &mut code) } == 0 {
            return Err(last("GetExitCodeProcess(normal target)"));
        }
        if code != 0 {
            return Err(ChildError::Unsupported("normal target exit was not zero"));
        }
        Ok(())
    }
}
fn last(context: &'static str) -> ChildError {
    ChildError::io(context, io::Error::last_os_error())
}
