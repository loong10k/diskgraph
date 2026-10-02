use std::fs::File;
use std::os::windows::io::AsRawHandle;

use diskgraph_core::BusinessError;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_ID_INFO, FILE_STANDARD_INFO,
    FILE_TYPE_DISK, FileBasicInfo, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx,
    GetFileType,
};

use crate::EngineError;

/// 已打开句柄的完整身份与版本；来源：Windows FileId/Basic/StandardInfo。
/// 读取会更新访问时间，因此指纹不包含 LastAccessTime。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowsFileState {
    pub(crate) volume: u64,
    id: [u8; 16],
    pub(crate) len: u64,
    creation: i64,
    modified: i64,
    changed: i64,
    attributes: u32,
    pub(crate) directory: bool,
    delete_pending: bool,
}

impl WindowsFileState {
    /// 从借用句柄查询状态；API/身份不可用时拒绝，不降级为截断的 file ID。
    pub(crate) fn capture(file: &File) -> Result<Self, EngineError> {
        let handle = file.as_raw_handle();
        // 安全性：句柄仍由 File 持有，三块输出有正确类型及精确大小。
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return Err(EngineError::Business(BusinessError::Unsupported));
        }
        let mut id = FILE_ID_INFO::default();
        let mut basic = FILE_BASIC_INFO::default();
        let mut standard = FILE_STANDARD_INFO::default();
        let queried = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&mut id as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            ) != 0
                && GetFileInformationByHandleEx(
                    handle,
                    FileBasicInfo,
                    (&mut basic as *mut FILE_BASIC_INFO).cast(),
                    std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                ) != 0
                && GetFileInformationByHandleEx(
                    handle,
                    FileStandardInfo,
                    (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                    std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
                ) != 0
        };
        if !queried || standard.EndOfFile < 0 {
            return Err(EngineError::Business(BusinessError::Unsupported));
        }
        Ok(Self {
            volume: id.VolumeSerialNumber,
            id: id.FileId.Identifier,
            len: standard.EndOfFile as u64,
            creation: basic.CreationTime,
            modified: basic.LastWriteTime,
            changed: basic.ChangeTime,
            attributes: basic.FileAttributes,
            directory: standard.Directory,
            delete_pending: standard.DeletePending,
        })
    }

    /// 判断句柄是否暴露云占位/离线属性；返回 true 时不得申请数据访问。
    pub(crate) fn placeholder(&self) -> bool {
        self.attributes
            & (FILE_ATTRIBUTE_OFFLINE
                | FILE_ATTRIBUTE_RECALL_ON_OPEN
                | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
    }

    /// 检查属性句柄是否可继续使用；目录重解析与待删除对象明确拒绝。
    pub(crate) fn validate(&self, directory: bool) -> Result<(), EngineError> {
        if self.directory != directory {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
        if self.delete_pending {
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        if self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || (directory && self.placeholder())
        {
            return Err(EngineError::Business(BusinessError::Unsupported));
        }
        Ok(())
    }
}
