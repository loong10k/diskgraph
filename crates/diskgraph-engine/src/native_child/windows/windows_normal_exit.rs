//! 正常完成许可仅查询原 Job；不请求终止，不通过关闭句柄制造退出。

use std::io;
use std::ptr::null_mut;
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject,
};

use super::super::{ChildError, ControlWriteStatus};
use super::owned_handle::OwnedHandle;

/// 已实际关闭控制端的本地事实与真实 Job 空状态查询。来源：Win32 Job accounting、原控制管道 Closed 完成语义。
pub(super) struct WindowsNormalExit {
    control_closed: bool,
}

impl WindowsNormalExit {
    /// 创建尚未具备正常完成资格的记录。参数：无。返回：控制关闭事实为 false 的实例。
    pub(super) fn new() -> Self {
        Self {
            control_closed: false,
        }
    }

    /// 仅记录同 owner 原管道已实际返回的 Closed。参数：result 为未经改写的控制操作结果。返回：无；Pending、Written 与所有错误均不能供给关闭许可。
    pub(super) fn record_control(&mut self, result: &Result<ControlWriteStatus, ChildError>) {
        if matches!(result, Ok(ControlWriteStatus::Closed)) {
            self.control_closed = true;
        }
    }

    /// 在完整传输和 leader 退出后查询原 Job 活动数。参数：job 是仍保留的原 Job owner，leader_exited 为原真实 poll 事实，pipes_eof 为两条真实 EOF。返回：全部完成且 ActiveProcesses 为零时 true，活动为 false，丢失 owner 或原查询失败为错误。
    pub(super) fn poll(
        &self,
        job: Option<&OwnedHandle>,
        leader_exited: bool,
        pipes_eof: bool,
    ) -> Result<bool, ChildError> {
        let job = job.ok_or(ChildError::Unsupported(
            "normal-exit Job owner already cleaned",
        ))?;
        if !self.control_closed || !leader_exited || !pipes_eof {
            return Ok(false);
        }
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe {
            QueryInformationJobObject(
                job.as_raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(ChildError::io(
                "QueryInformationJobObject(normal exit)",
                io::Error::last_os_error(),
            ));
        }
        Ok(accounting.ActiveProcesses == 0)
    }
}
