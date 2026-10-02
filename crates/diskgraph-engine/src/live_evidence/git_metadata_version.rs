#[cfg(unix)]
use std::fs::Metadata;
use std::time::SystemTime;

/// 原生文件身份、内容版本与高精度修改时间；来源：Unix stat 或 Windows FileId/BasicInfo。
pub(super) struct GitMetadataVersion {
    #[cfg(unix)]
    initial: Metadata,
    #[cfg(windows)]
    initial: crate::windows_file_state::WindowsFileState,
    modified: SystemTime,
}

impl GitMetadataVersion {
    /// 保存 Unix 已打开句柄的原始版本。参数：metadata 为 nofollow 打开后的 fstat。返回：版本或时间不可用错误。
    #[cfg(unix)]
    pub(super) fn from_metadata(metadata: &Metadata) -> Result<Self, String> {
        if !metadata.is_file() {
            return Err("git metadata is not a regular file".into());
        }
        Ok(Self {
            initial: metadata.clone(),
            modified: metadata
                .modified()
                .map_err(|error| format!("git metadata modified time: {error}"))?,
        })
    }

    /// 对比 Unix 文件身份、长度、mtime 和 ctime 的原生精度。参数：metadata 为当前句柄 fstat。返回：版本是否相同。
    #[cfg(unix)]
    pub(super) fn matches_metadata(&self, metadata: &Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        let old = &self.initial;
        metadata.is_file()
            && old.dev() == metadata.dev()
            && old.ino() == metadata.ino()
            && old.len() == metadata.len()
            && old.mtime() == metadata.mtime()
            && old.mtime_nsec() == metadata.mtime_nsec()
            && old.ctime() == metadata.ctime()
            && old.ctime_nsec() == metadata.ctime_nsec()
    }

    /// 保存 Windows 完整 FileId 与版本。参数：state 为安全租约状态，modified 为同句柄修改时间。返回：版本记录。
    #[cfg(windows)]
    pub(super) fn from_windows_state(
        state: &crate::windows_file_state::WindowsFileState,
        modified: SystemTime,
    ) -> Self {
        Self {
            initial: state.clone(),
            modified,
        }
    }

    /// 对比 Windows 完整文件身份和版本。参数：state 为重开后的安全租约状态。返回：版本是否相同。
    #[cfg(windows)]
    pub(super) fn matches_windows_state(
        &self,
        state: &crate::windows_file_state::WindowsFileState,
    ) -> bool {
        self.initial == *state
    }

    /// 返回原生高精度修改时间以恢复私有 index 的 racy-git 水位。参数：无。返回：首次句柄观察到的修改时间。
    pub(super) fn modified(&self) -> SystemTime {
        self.modified
    }

    /// 比较两次独立捕获的原生身份版本。参数：current 为末段重开的版本。返回：文件身份和版本是否相同。
    pub(super) fn same_initial(&self, current: &Self) -> bool {
        #[cfg(unix)]
        {
            self.matches_metadata(&current.initial)
        }
        #[cfg(windows)]
        {
            self.matches_windows_state(&current.initial)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = current;
            false
        }
    }
}
