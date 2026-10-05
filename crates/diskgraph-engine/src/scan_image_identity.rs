use crate::EngineError;
use diskgraph_core::BusinessError;
use std::fs::File;

/// 同一镜像句柄的完整身份与观察版本，不使用路径重新定位或猜测卷/文件编号。
/// 来源：原生 Rust PF-06、Unix fstat 与 Windows FileIdInfo/FileBasicInfo。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScanImageIdentity {
    #[cfg(unix)]
    Unix {
        device: u64,
        inode: u64,
        bytes: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    },
    #[cfg(windows)]
    Windows {
        volume: u64,
        file_id: [u8; 16],
        bytes: u64,
        created: i64,
        modified: i64,
        changed: i64,
    },
}

impl ScanImageIdentity {
    /// 参数：file 是宿主已打开的原镜像；返回：完整句柄身份或原 I/O/能力拒绝。
    pub(crate) fn capture(file: &File) -> Result<Self, EngineError> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(BusinessError::InvalidArgument.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.ino() == 0 {
                return Err(BusinessError::Unsupported.into());
            }
            Ok(Self::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
                bytes: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo,
                GetFileInformationByHandleEx,
            };
            let mut identity = FILE_ID_INFO::default();
            let mut basic = FILE_BASIC_INFO::default();
            // 安全性：原 File 在调用期间持有 HANDLE，两个缓冲均为对应 Windows ABI 的完整对象。
            if unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileIdInfo,
                    (&raw mut identity).cast(),
                    std::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileBasicInfo,
                    (&raw mut basic).cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            if identity.FileId.Identifier == [0; 16] {
                return Err(BusinessError::Unsupported.into());
            }
            Ok(Self::Windows {
                volume: identity.VolumeSerialNumber,
                file_id: identity.FileId.Identifier,
                bytes: metadata.len(),
                created: basic.CreationTime,
                modified: basic.LastWriteTime,
                changed: basic.ChangeTime,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(BusinessError::Unsupported.into())
        }
    }
}
