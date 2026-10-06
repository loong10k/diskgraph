//! 单请求原生启动事实记录；保留观察句柄直到被测 owner 已完成清理。

use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, WaitForSingleObject};

use super::super::ChildError;
use super::owned_handle::OwnedHandle;

/// 真实创建计数与只读 Job／leader 句柄；来源：Win32 DuplicateHandle、Job 活动计数和真实 wait。
pub(super) struct WindowsControlSpawnRecord {
    creations: usize,
    job: Option<OwnedHandle>,
    leader: Option<OwnedHandle>,
    failure: Option<ChildError>,
}

impl WindowsControlSpawnRecord {
    /// 创建尚无原生事实的记录。参数：无。返回：空记录，不能单独证明创建或清理。
    pub(super) fn new() -> Self {
        Self {
            creations: 0,
            job: None,
            leader: None,
            failure: None,
        }
    }

    /// 在原 owner 安装后复制真实句柄。参数：job、leader 是仍存活的原句柄。返回：无；错误留存，不改变产品结果。
    pub(super) fn record(&mut self, job: HANDLE, leader: HANDLE) {
        self.creations += 1;
        match duplicate(job).and_then(|job| duplicate(leader).map(|leader| (job, leader))) {
            Ok((job, leader)) => {
                self.job = Some(job);
                self.leader = Some(leader);
                self.failure = None;
            }
            Err(error) => self.failure = Some(error),
        }
    }

    /// 返回实际创建次数。参数：无。返回：本请求记录到的 Job／leader 对数。
    pub(super) fn creations(&self) -> usize {
        self.creations
    }

    /// 持复制句柄观察清理事实。参数：无。返回：leader 已退出且 Job 活动数为零，或真实查询错误。
    pub(super) fn terminated_and_empty(&self) -> Result<bool, ChildError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let job = self
            .job
            .as_ref()
            .ok_or(ChildError::Unsupported("no actual Job witness"))?;
        let leader = self
            .leader
            .as_ref()
            .ok_or(ChildError::Unsupported("no actual leader witness"))?;
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
                "QueryInformationJobObject(control test)",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(
            unsafe { WaitForSingleObject(leader.as_raw(), 0) } == WAIT_OBJECT_0
                && accounting.ActiveProcesses == 0,
        )
    }
}

fn duplicate(handle: HANDLE) -> Result<OwnedHandle, ChildError> {
    let mut copied = null_mut();
    let process = unsafe { GetCurrentProcess() };
    if unsafe {
        DuplicateHandle(
            process,
            handle,
            process,
            &mut copied,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(ChildError::io(
            "DuplicateHandle(control test)",
            std::io::Error::last_os_error(),
        ));
    }
    OwnedHandle::from_raw(copied, "DuplicateHandle(control test owner)")
}
