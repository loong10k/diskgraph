//! 原WindowsChild的有限处置入口；嵌套模块保持全部原owner字段私有，旧cleanup/Drop不变。
use super::super::cleanup_progress::CleanupProgress;
use super::WindowsChild;
use crate::native_child::ChildError;
use std::ptr::null_mut;
use std::time::Instant;
use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
    QueryInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

impl WindowsChild {
    /// 参数：deadline为调用者原处置绝对期限；返回：一次非阻塞观察，Pending/Err保留原owner。
    /// Wait0/QueryJob/管道FALSE查询都不等待；只有leader、全Job及全部I/O实际完成才释放。
    /// 该入口不保证单次内核调用的硬墙钟上限；旧cleanup/Drop和出生前准备仍可能阻塞。
    pub(crate) fn poll_cleanup(
        &mut self,
        deadline: Instant,
    ) -> Result<CleanupProgress, ChildError> {
        if self.cleaned {
            return Ok(CleanupProgress::Complete);
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        let job = self
            .job
            .as_ref()
            .ok_or(ChildError::Unsupported("poll cleanup Job missing"))?;
        if unsafe { TerminateJobObject(job.as_raw(), 1) } == 0 {
            return Err(super::last("TerminateJobObject(poll cleanup)"));
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        let leader_done = match self.process.as_ref() {
            Some(process) => {
                #[cfg(test)]
                let waited =
                    super::super::windows_cleanup_hooks::WindowsCleanupHooks::wait(|| unsafe {
                        WaitForSingleObject(process.as_raw(), 0)
                    });
                #[cfg(not(test))]
                let waited = unsafe { WaitForSingleObject(process.as_raw(), 0) };
                match waited {
                    WAIT_OBJECT_0 => true,
                    WAIT_TIMEOUT => false,
                    _ => return Err(super::last("WaitForSingleObject(poll cleanup)")),
                }
            }
            None if self.birth_phase.confirmed_unborn() => true,
            None => return Err(ChildError::Unsupported("unconfirmed birth leader missing")),
        };
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        #[cfg(test)]
        let queried = super::super::windows_cleanup_hooks::WindowsCleanupHooks::query(|| unsafe {
            QueryInformationJobObject(
                job.as_raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                null_mut(),
            )
        });
        #[cfg(not(test))]
        let queried = unsafe {
            QueryInformationJobObject(
                job.as_raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                null_mut(),
            )
        };
        if queried == 0 {
            return Err(super::last("QueryInformationJobObject(poll cleanup)"));
        }
        if Instant::now() >= deadline {
            return Ok(CleanupProgress::Pending);
        }
        let unborn = self.birth_phase.confirmed_unborn();
        let stdout_done = match self.stdout.as_mut() {
            Some(pipe) => pipe.poll_cleanup(deadline)? == CleanupProgress::Complete,
            None if unborn => true,
            None => return Err(ChildError::Unsupported("born stdout owner missing")),
        };
        let stderr_done = match self.stderr.as_mut() {
            Some(pipe) => pipe.poll_cleanup(deadline)? == CleanupProgress::Complete,
            None if unborn => true,
            None => return Err(ChildError::Unsupported("born stderr owner missing")),
        };
        let control_done = match self.control.as_mut() {
            Some(control) => control.poll_cleanup(deadline)? == CleanupProgress::Complete,
            None => true,
        };
        if Instant::now() >= deadline
            || !leader_done
            || accounting.ActiveProcesses != 0
            || !stdout_done
            || !stderr_done
            || !control_done
        {
            return Ok(CleanupProgress::Pending);
        }
        self.process.take();
        self.job.take();
        self.cleaned = true;
        Ok(CleanupProgress::Complete)
    }
}
