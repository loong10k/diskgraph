use super::git_private_allocation::GitPrivateAllocation;
use super::git_private_directory::GitPrivateDirectory;
use std::path::PathBuf;

/// 真实Git私有目录的只读身份见证；来源：生产GitPrivateAllocation，不持删除阻止句柄。
pub(crate) struct ProbeDirectoryWitness {
    path: PathBuf,
    identity: GitPrivateAllocation,
}

impl ProbeDirectoryWitness {
    /// 从真实生产目录捕获原生身份；不保留文件句柄，不改变删除或共享语义。
    pub(super) fn capture(directory: &GitPrivateDirectory) -> Result<Self, String> {
        let path = directory.path().to_owned();
        let identity = GitPrivateAllocation::capture(&path)?;
        Ok(Self { path, identity })
    }

    /// 观察当前名称仍是否指向原生同一目录；不存在为false，其他查询错误不能当删除成功。
    pub(crate) fn same_directory_exists(&self) -> Result<bool, String> {
        match std::fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
            Ok(_) => GitPrivateAllocation::capture(&self.path)
                .map(|current| current.is_directory() && self.identity.same_identity(&current)),
        }
    }

    /// 观察原名称实际消失；不能把被替换目录解释为已删除。
    pub(crate) fn absent(&self) -> Result<bool, String> {
        match std::fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.to_string()),
            Ok(_) => Ok(false),
        }
    }

    /// 仅finally救援：调用者必须先实际回收fixture child；保留陌生替换目录，不作为生产成功证据。
    pub(crate) fn rescue_after_child_reaped(&self) -> Result<(), String> {
        if self.absent()? {
            return Ok(());
        }
        if !self.same_directory_exists()? {
            return Err("private Git rescue identity changed; foreign directory retained".into());
        }
        std::fs::remove_dir_all(&self.path).map_err(|error| error.to_string())
    }
}
