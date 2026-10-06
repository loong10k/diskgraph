//! 首次 I/O 前接管原 Job；后续准备失败均保留外槽。
use super::super::windows_birth_phase::WindowsBirthPhase;
use super::super::{owned_handle::OwnedHandle, windows_normal_exit::WindowsNormalExit};
use super::{WindowsChild, last};
use crate::native_child::ChildError;
use std::ptr::null;
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};

impl WindowsChild {
    /// 参数：空的 catch 外槽；返回：原 Job 创建/设置结果，首次 I/O 前已持有确认未出生状态。
    pub(super) fn prepare_job_into(owner: &mut Option<Self>) -> Result<(), ChildError> {
        if owner.is_some() {
            return Err(ChildError::Unsupported("birth owner slot already occupied"));
        }
        let job = OwnedHandle::from_raw(
            unsafe { CreateJobObjectW(null(), null()) },
            "CreateJobObjectW",
        )?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.as_raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(last("SetInformationJobObject(KILL_ON_JOB_CLOSE)"));
        }
        *owner = Some(Self {
            job: Some(job),
            process: None,
            stdout: None,
            stderr: None,
            control: None,
            birth_phase: WindowsBirthPhase::Prepared,
            normal_exit: WindowsNormalExit::new(),
            exit: None,
            cleaned: false,
            _image_guard: None,
        });
        Ok(())
    }
}
