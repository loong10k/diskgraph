use crate::scan_image_identity::ScanImageIdentity;
use crate::scan_worker_host_config::MAX_IMAGE_BYTES;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::time::Instant;

/// Windows原镜像的禁止写/删除共享读取租约及独立摘要核验；来源：PF-06/NtOpenFile，无Java对应。
/// 本材料不授予执行许可，不证明既有可写映射、实际加载器或DLL搜索已安全。
pub(crate) struct WindowsScanImageLease {
    file: File,
    identity: ScanImageIdentity,
    binding: crate::windows_scan_image_binding::WindowsScanImageBinding,
}

impl WindowsScanImageLease {
    /// 参数：path为受信本地部署来源；返回：先属性核验再相对原句柄取得的读取File。
    /// 路径只打开属性，拒绝占位/特殊对象后才获取数据访问；仍不授予加载或执行许可。
    pub(crate) fn open_source(path: &std::path::Path) -> Result<File, EngineError> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
        };
        let source = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let state = crate::windows_file_state::WindowsFileState::capture(&source)?;
        state.validate(false)?;
        if state.placeholder() {
            return Err(BusinessError::Unsupported.into());
        }
        let identity = ScanImageIdentity::capture(&source)?;
        let file = reopen_read(&source)?;
        if ScanImageIdentity::capture(&file)? != identity
            || ScanImageIdentity::capture(&source)? != identity
        {
            return Err(BusinessError::Conflict.into());
        }
        Ok(file)
    }

    /// 参数：source为原可信宿主句柄、expected为独立预期、deadline/checkpoint为同次准入预算。
    /// 返回：同对象只读租约；所有失败关闭本次新句柄，不重开任何用户路径或执行镜像。
    pub(crate) fn prepare(
        source: File,
        expected: &ScanWorkerHostConfig,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check(deadline, checkpoint)?;
        let state = crate::windows_file_state::WindowsFileState::capture(&source)?;
        state.validate(false)?;
        if state.placeholder() {
            return Err(BusinessError::Unsupported.into());
        }
        let original = ScanImageIdentity::capture(&source)?;
        let mut file = reopen_read(&source)?;
        let identity = ScanImageIdentity::capture(&file)?;
        if original != identity {
            return Err(BusinessError::Conflict.into());
        }
        let state = crate::windows_file_state::WindowsFileState::capture(&file)?;
        state.validate(false)?;
        if state.placeholder() {
            return Err(BusinessError::Unsupported.into());
        }
        let length = file.metadata()?.len();
        if length > MAX_IMAGE_BYTES {
            return Err(BusinessError::BudgetExceeded.into());
        }
        if length != expected.expected_bytes {
            return Err(BusinessError::Conflict.into());
        }
        let binding = crate::windows_scan_image_binding::WindowsScanImageBinding::prepare(
            &file, &identity, deadline, checkpoint,
        )?;
        file.seek(SeekFrom::Start(0))?;
        let mut block = [0_u8; 64 * 1024];
        let mut remaining = length;
        let mut digest = Sha256::new();
        while remaining != 0 {
            check(deadline, checkpoint)?;
            let admitted = remaining.min(block.len() as u64) as usize;
            let count = match file.read(&mut block[..admitted]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                return Err(BusinessError::Conflict.into());
            }
            digest.update(&block[..count]);
            remaining -= count as u64;
        }
        let actual: [u8; 32] = digest.finalize().into();
        if actual != expected.expected_sha256 {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, checkpoint)?;
        if ScanImageIdentity::capture(&source)? != identity {
            return Err(BusinessError::Conflict.into());
        }
        let lease = Self {
            file,
            identity,
            binding,
        };
        lease.validate(deadline, checkpoint)?;
        Ok(lease)
    }

    /// 参数：原准入期限及检查点；返回：原句柄仍为已核身份版本，不提供按路径加载资格。
    pub(crate) fn validate(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        check(deadline, checkpoint)?;
        if ScanImageIdentity::capture(&self.file)? != self.identity {
            return Err(BusinessError::Conflict.into());
        }
        self.binding
            .validate(&self.file, &self.identity, deadline, checkpoint)
    }
}

fn reopen_read(source: &File) -> Result<File, EngineError> {
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_OPEN_NO_RECALL, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtOpenFile,
    };
    use windows_sys::Win32::Foundation::{
        INVALID_HANDLE_VALUE, OBJ_DONT_REPARSE, RtlNtStatusToDosError, UNICODE_STRING,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_SHARE_READ};
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let empty = UNICODE_STRING::default();
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: source.as_raw_handle(),
        ObjectName: &empty,
        Attributes: OBJ_DONT_REPARSE,
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut handle = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // 同步调用借原File寿命；有效返回句柄在错误投影前接管，失败不按名称回退。
    let status = unsafe {
        NtOpenFile(
            &mut handle,
            FILE_GENERIC_READ,
            &attributes,
            &mut status_block,
            FILE_SHARE_READ,
            FILE_OPEN_NO_RECALL | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        )
    };
    let file = (!handle.is_null() && handle != INVALID_HANDLE_VALUE)
        .then(|| unsafe { File::from_raw_handle(handle) });
    if status != 0 {
        return Err(
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) as i32 }).into(),
        );
    }
    file.ok_or_else(|| io::Error::other("native image reopen returned no valid handle").into())
}

fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}
