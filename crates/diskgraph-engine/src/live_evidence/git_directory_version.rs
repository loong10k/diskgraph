use std::fs::File;

/// 不持有句柄的目录身份与完整版本。来源：Unix fstat 或 Windows FileId/BasicInfo。
/// 祖先只比较完整身份，叶目录比较身份、长度与原生时间；不将外部祖先 mtime 变化当成冲突。
#[derive(Eq, PartialEq)]
pub(super) struct GitDirectoryVersion {
    #[cfg(unix)]
    values: [u64; 7],
    #[cfg(windows)]
    state: crate::windows_file_state::WindowsFileState,
    #[cfg(windows)]
    identity: (u64, [u8; 16]),
}

impl GitDirectoryVersion {
    /// 保存已打开普通目录的不可变版本。参数：file 为 no-follow 租约中的目录。
    /// 返回：完整身份版本；类型、平台查询或身份无法可靠验证时明确拒绝。
    pub(super) fn capture(file: &File) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata().map_err(|error| error.to_string())?;
            if !metadata.is_dir() {
                return Err("git metadata is not a directory".into());
            }
            Ok(Self {
                // 有符号时间转换保留原始位模式，仅用于完整相等比较。
                values: [
                    metadata.dev(),
                    metadata.ino(),
                    metadata.len(),
                    metadata.mtime() as u64,
                    metadata.mtime_nsec() as u64,
                    metadata.ctime() as u64,
                    metadata.ctime_nsec() as u64,
                ],
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
            };
            let state = crate::windows_file_state::WindowsFileState::capture(file)
                .map_err(|error| error.to_string())?;
            state.validate(true).map_err(|error| error.to_string())?;
            let mut id = FILE_ID_INFO::default();
            if unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileIdInfo,
                    (&mut id as *mut FILE_ID_INFO).cast(),
                    std::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            } == 0
                || id.VolumeSerialNumber != state.volume
            {
                return Err("unsupported Git directory identity".into());
            }
            Ok(Self {
                state,
                identity: (id.VolumeSerialNumber, id.FileId.Identifier),
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = file;
            Err("unsupported Git directory platform".into())
        }
    }

    /// 对照已索引目录身份。参数：expected 为持久 revision 派生身份；返回：同一原生对象时 true。
    pub(super) fn matches_indexed(
        &self,
        expected: &super::git_indexed_directory::GitIndexedDirectory,
    ) -> bool {
        #[cfg(unix)]
        {
            (self.values[0], self.values[1]) == expected.unix
        }
        #[cfg(windows)]
        {
            let observation =
                self.state
                    .observation(0, 0, diskgraph_core::WindowsTreeAlignment::Matched);
            (self.identity.0, self.identity.1, observation.creation_time) == expected.windows
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = expected;
            false
        }
    }

    /// 比较两次解析所得的完整祖先身份。参数：current 为新捕获的目录版本。
    /// 返回：同一 Unix dev/ino 或 Windows volume/128bit ID 时 true，忽略祖先无关版本变化。
    pub(super) fn same_identity(&self, current: &Self) -> bool {
        #[cfg(unix)]
        {
            self.values[..2] == current.values[..2]
        }
        #[cfg(windows)]
        {
            self.identity == current.identity
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = current;
            false
        }
    }
}
