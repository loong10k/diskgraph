use crate::EngineError;
use crate::scan_image_identity::ScanImageIdentity;
use crate::windows_native_scan_root::WindowsNativeScanRoot;
use crate::windows_path_plan::WindowsPathPlan;
use diskgraph_core::BusinessError;
use std::cell::RefCell;
use std::ffi::OsString;
use std::fs::File;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::time::Instant;
use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;

/// 原镜像最终名称与非重解析父链租约；来源：PF-06/Windows 原生 API，无 Java 对应。
/// 保留原 namespace，不冻结新增 DLL，也不单独授予 CreateProcess 执行资格。
pub(crate) struct WindowsScanImageBinding {
    path: PathBuf,
    parent: WindowsNativeScanRoot,
}

impl WindowsScanImageBinding {
    /// 参数：file/identity 是已保留镜像及完整版本，deadline/checkpoint 是原请求预算。
    /// 返回：同对象末叶及全部父名称的保留绑定；未知路径空间或变更直接拒绝。
    pub(crate) fn prepare(
        file: &File,
        identity: &ScanImageIdentity,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        // RefCell 只在单线程同步回调内依次借用，绝不复制检查点或重置期限。
        let checkpoint = RefCell::new(checkpoint);
        let check = || {
            (checkpoint.borrow_mut())()?;
            if Instant::now() >= deadline {
                return Err(BusinessError::BudgetExceeded.into());
            }
            Ok(())
        };
        check()?;
        let mut wide = [0_u16; 32768];
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                wide.as_mut_ptr(),
                wide.len() as u32,
                0,
            )
        };
        // 必须先保存本次原生错误，再执行可能调用系统 API 的用户检查。
        let error = (length == 0).then(std::io::Error::last_os_error);
        check()?;
        if let Some(error) = error {
            return Err(error.into());
        }
        if length as usize >= wide.len() {
            return Err(BusinessError::BudgetExceeded.into());
        }
        if wide[..length as usize].contains(&0) {
            return Err(BusinessError::InvalidArgument.into());
        }
        let path = PathBuf::from(OsString::from_wide(&wide[..length as usize]));
        WindowsPathPlan::for_root(&path)?;
        let parent_path = path.parent().ok_or(BusinessError::InvalidArgument)?;
        let parent = WindowsNativeScanRoot::open(parent_path, &check)?;
        let binding = Self { path, parent };
        binding.validate_checked(file, identity, &check)?;
        Ok(binding)
    }

    /// 参数：原镜像、原完整版本与同次期限/检查；返回：当前末叶仍绑定同对象版本。
    pub(crate) fn validate(
        &self,
        file: &File,
        identity: &ScanImageIdentity,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let checkpoint = RefCell::new(checkpoint);
        let check = || {
            (checkpoint.borrow_mut())()?;
            if Instant::now() >= deadline {
                return Err(BusinessError::BudgetExceeded.into());
            }
            Ok(())
        };
        self.validate_checked(file, identity, &check)
    }

    /// 核验原进程报告的Win32镜像名称，不按该名称重开任何文件。
    /// 参数：file/identity为原材料，process为借用句柄，deadline/checkpoint沿原请求。
    /// 返回：原父链、原句柄版本和报告名称一致；不证明映射字节或授予执行。
    pub(crate) fn verify_process_name(
        &self,
        file: &File,
        identity: &ScanImageIdentity,
        process: std::os::windows::io::BorrowedHandle<'_>,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
        self.validate(file, identity, deadline, checkpoint)?;
        let mut wide = [0_u16; 32768];
        let mut length = wide.len() as u32;
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let queried = unsafe {
            QueryFullProcessImageNameW(process.as_raw_handle(), 0, wide.as_mut_ptr(), &mut length)
        };
        // 必须先保存本次Win32原错，再执行可能改变线程last-error的业务检查。
        let error = (queried == 0).then(std::io::Error::last_os_error);
        checkpoint()?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        if let Some(error) = error {
            return Err(error.into());
        }
        let length = length as usize;
        if length == 0 || length >= wide.len() || wide[length] != 0 || wide[..length].contains(&0) {
            return Err(BusinessError::InvalidArgument.into());
        }
        let actual = PathBuf::from(OsString::from_wide(&wide[..length]));
        let reported = WindowsPathPlan::for_root(&actual)?;
        let original = WindowsPathPlan::for_root(&self.path)?;
        if reported.drive_root != original.drive_root || reported.components != original.components
        {
            return Err(BusinessError::Conflict.into());
        }
        // 只从持续持有的原父句柄复核原叶，不从进程报告的可变路径取得新身份。
        self.validate(file, identity, deadline, checkpoint)
    }

    fn validate_checked(
        &self,
        file: &File,
        identity: &ScanImageIdentity,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        check()?;
        let name = self
            .path
            .file_name()
            .ok_or(BusinessError::InvalidArgument)?;
        let leaf = self.parent.open_leaf_attributes(name, check)?;
        check()?;
        let current = ScanImageIdentity::capture(&leaf)?;
        check()?;
        if &current != identity {
            return Err(BusinessError::Conflict.into());
        }
        let held = ScanImageIdentity::capture(file)?;
        check()?;
        if &held != identity {
            return Err(BusinessError::Conflict.into());
        }
        self.parent.validate_root(check)?;
        check()
    }
}
