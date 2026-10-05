use std::fs::File;
use std::os::windows::io::AsRawHandle;

use diskgraph_core::{BusinessError, FileIdentity, WindowsFileObservation, WindowsTreeAlignment};
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
    /// 参数：other 为同次原有捕获状态；不读取句柄或复制字段值。
    /// 返回：完整 Eq 字段的差异位：卷、ID、EOF、创建、写入、变化、属性、类型、删除依次为 bit0..8。
    #[cfg(test)]
    pub(crate) fn changed_mask(&self, other: &Self) -> u16 {
        u16::from(self.volume != other.volume)
            | (u16::from(self.id != other.id) << 1)
            | (u16::from(self.len != other.len) << 2)
            | (u16::from(self.creation != other.creation) << 3)
            | (u16::from(self.modified != other.modified) << 4)
            | (u16::from(self.changed != other.changed) << 5)
            | (u16::from(self.attributes != other.attributes) << 6)
            | (u16::from(self.directory != other.directory) << 7)
            | (u16::from(self.delete_pending != other.delete_pending) << 8)
    }

    /// 为扫描在每项原生查询前后检查任务条件；旧内容 capture 保持原行为。
    /// 参数：file 为保留的属性句柄，check 为原期限/取消/授权/fence 检查。
    /// 返回：外层传播 check 错误，内层返回完整属性或原生 unsupported，防止撤权转 Gap。
    pub(crate) fn capture_checked(
        file: &File,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Result<Self, EngineError>, EngineError> {
        let handle = file.as_raw_handle();
        if checked_native(check, || unsafe { GetFileType(handle) })? != FILE_TYPE_DISK {
            return Ok(Err(BusinessError::Unsupported.into()));
        }
        let mut id = FILE_ID_INFO::default();
        let mut basic = FILE_BASIC_INFO::default();
        let mut standard = FILE_STANDARD_INFO::default();
        // 缓冲区与借用句柄在各同步调用期间有效；每一步之后优先传播任务失效。
        let identity = checked_native(check, || unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&mut id as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        })?;
        if identity == 0 {
            return Ok(Err(BusinessError::Unsupported.into()));
        }
        let version = checked_native(check, || unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        })?;
        if version == 0 {
            return Ok(Err(BusinessError::Unsupported.into()));
        }
        let size = checked_native(check, || unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileStandardInfo,
                (&mut standard as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        })?;
        if size == 0 || standard.EndOfFile < 0 {
            return Ok(Err(BusinessError::Unsupported.into()));
        }
        Ok(Ok(Self {
            volume: id.VolumeSerialNumber,
            id: id.FileId.Identifier,
            len: standard.EndOfFile as u64,
            creation: basic.CreationTime,
            modified: basic.LastWriteTime,
            changed: basic.ChangeTime,
            attributes: basic.FileAttributes,
            directory: standard.Directory,
            delete_pending: standard.DeletePending,
        }))
    }

    /// 比较根租约的完整身份与目录安全状态；允许目录时间和普通属性变化。
    /// 参数：current 为保留句柄或其当前名称绑定重新读取的状态。
    /// 返回：卷、128 位 ID、创建时间、目录类型及非重解析/未删除条件保持时 true。
    pub(crate) fn matches_scan_root(&self, current: &Self) -> bool {
        self.volume == current.volume
            && self.id == current.id
            && self.creation == current.creation
            && self.directory
            && current.directory
            && current.attributes & 0x10 != 0
            && !current.reparse()
            && !current.delete_pending
    }

    /// 判断属性对象是否为重解析点；不沿其目标解析。
    /// 参数：无，使用当前句柄的属性位。
    /// 返回：FILE_ATTRIBUTE_REPARSE_POINT 存在时 true。
    pub(crate) fn reparse(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// 无损投影辅助身份；高 64 位非零时保留旧字段 unknown。
    /// 参数：无，使用原生卷与完整 ID。
    /// 返回：能够无损表达的兼容身份，或 None；不截断/散列。
    pub(crate) fn legacy_identity(&self) -> Option<FileIdentity> {
        if self.id[8..].iter().any(|byte| *byte != 0) {
            return None;
        }
        Some(FileIdentity {
            volume_id: format!("windows-volume-{:016x}", self.volume),
            file_id: u64::from_le_bytes(self.id[..8].try_into().ok()?),
        })
    }

    /// 将句柄原始属性转为独立采样纯值；不混入旧树尺寸/时间。
    /// 参数：started/finished 为采样窗口，alignment 为已验证的树对齐状态。
    /// 返回：包含完整 ID、原生 ticks 和本次长度的观测；调用方须验证窗口。
    pub(crate) fn observation(
        &self,
        started: u64,
        finished: u64,
        alignment: WindowsTreeAlignment,
    ) -> WindowsFileObservation {
        WindowsFileObservation {
            volume: self.volume,
            file_id: self.id,
            length: self.len,
            creation_time: self.creation,
            last_write_time: self.modified,
            change_time: self.changed,
            attributes: self.attributes,
            directory: self.directory,
            delete_pending: self.delete_pending,
            capture_started_unix_ms: started,
            capture_finished_unix_ms: finished,
            tree_alignment: alignment,
        }
    }

    /// 从借用句柄查询状态；API/身份不可用时拒绝，不降级为截断的 file ID。
    /// 参数：file 为调用方持有的原生属性或数据句柄。
    /// 返回：卷、完整 file ID、版本及属性；API 或身份不可验证时返回 unsupported。
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
    /// 参数：无。
    /// 返回：观察到离线或回调物化属性时 true；此时不得申请数据访问。
    pub(crate) fn placeholder(&self) -> bool {
        self.attributes
            & (FILE_ATTRIBUTE_OFFLINE
                | FILE_ATTRIBUTE_RECALL_ON_OPEN
                | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
    }

    /// 检查属性句柄是否可继续使用；目录重解析与待删除对象明确拒绝。
    /// 参数：directory 指定期望对象类型。
    /// 返回：类型、删除状态与重解析门禁通过，或对应业务错误。
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

fn checked_native<T>(
    check: &dyn Fn() -> Result<(), EngineError>,
    native: impl FnOnce() -> T,
) -> Result<T, EngineError> {
    check()?;
    let result = native();
    check()?;
    Ok(result)
}
