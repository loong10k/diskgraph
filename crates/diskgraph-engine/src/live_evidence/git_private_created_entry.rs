use super::windows_git_child_open::WindowsGitChildOpen;
use std::fs::File;
use std::path::{Path, PathBuf};

/// 内核创建之前准备的私有子项恢复责任；不根据路径重新收养对象。
/// 来源：Windows NtCreateFile 与私有 Git 原身份恢复合同；原生 Rust 实现。
pub(super) struct GitPrivateCreatedEntry {
    pub(super) path: PathBuf,
    pub(super) file: Option<File>,
    pub(super) directory: bool,
    pub(super) confirmed: bool,
}

impl GitPrivateCreatedEntry {
    /// 参数：path 为已准入路径，directory 为创建类型。返回：尚未执行内核创建的恢复槽位。
    pub(super) fn prepare(path: &Path, directory: bool) -> Self {
        Self {
            path: path.to_owned(),
            file: None,
            directory,
            confirmed: false,
        }
    }

    /// 参数：parent 为已验证的原父句柄。返回：独占创建结果；任意有效句柄留在原槽位。
    pub(super) fn create(&mut self, parent: &File) -> Result<(), String> {
        let name = self
            .path
            .file_name()
            .ok_or("private Git created entry name missing")?;
        WindowsGitChildOpen::open_into(
            parent,
            name,
            true,
            self.directory,
            !self.directory,
            windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ,
            &mut self.file,
        )?;
        self.confirmed = true;
        Ok(())
    }
}
