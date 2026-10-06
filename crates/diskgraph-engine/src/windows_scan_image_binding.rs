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
