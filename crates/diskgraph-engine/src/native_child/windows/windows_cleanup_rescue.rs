//! 故障测试的独立Job/leader见证；救援不能作为原owner成功的证据。
use super::owned_handle::OwnedHandle;
use super::windows_child::WindowsChild;
use crate::native_child::ChildError;
use std::ptr::null_mut;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

/// catch外独立救援句柄，直到真实leader wait及Job0才允许释放；来源：Win32原生测试。
pub(super) struct WindowsCleanupRescue {
    job: OwnedHandle,
    leader: OwnedHandle,
}
impl WindowsCleanupRescue {
    /// 参数：真实child；返回独立复制句柄，只读见证不替代原owner责任。
    pub(super) fn capture(child: &WindowsChild) -> Result<Self, ChildError> {
        let job = child.duplicate_job_for_test()?;
        let leader = child.duplicate_leader_for_test()?;
        Ok(Self { job, leader })
    }
    /// 查询独立Job的实际活动数，不调用生产cleanup或测试hook。
    pub(super) fn active(&self) -> Result<u32, ChildError> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        if unsafe {
            QueryInformationJobObject(
                self.job.as_raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(native("QueryInformationJobObject(rescue witness)"));
        }
        Ok(accounting.ActiveProcesses)
    }
    /// 只读实际leader是否已退出，错误不能等价为退出。
    pub(super) fn waited(&self) -> Result<bool, ChildError> {
        match unsafe { WaitForSingleObject(self.leader.as_raw(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(native("WaitForSingleObject(rescue witness)")),
        }
    }
    /// finally救援实际Job；期限固定，任何失败保留本对象由外层显式泄留而非Drop冒充完成。
    pub(super) fn finish(&self) -> Result<(), ChildError> {
        if self.active()? != 0 && unsafe { TerminateJobObject(self.job.as_raw(), 1) } == 0 {
            return Err(native("TerminateJobObject(rescue)"));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.waited()? && self.active()? == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ChildError::io(
                    "cleanup rescue deadline",
                    std::io::Error::from(std::io::ErrorKind::TimedOut),
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
fn native(context: &str) -> ChildError {
    ChildError::io(context, std::io::Error::last_os_error())
}
