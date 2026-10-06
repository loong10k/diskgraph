use super::git_private_allocation::GitPrivateAllocation;
use super::git_private_capacity::GitPrivateCapacity;
use std::path::PathBuf;

/// 可随原child移交Recovery的唯一目录payload；来源：Git私有视图原身份、容量及删除合同。
pub(crate) struct GitPrivateDirectoryOwner {
    pub(super) path: PathBuf,
    pub(super) cleaned: bool,
    pub(super) capacity: Option<GitPrivateCapacity>,
    #[cfg(windows)]
    pub(super) windows_cleanup: Option<super::windows_git_cleanup::WindowsGitCleanup>,
    pub(super) root_identity: Option<GitPrivateAllocation>,
}
impl GitPrivateDirectoryOwner {
    /// 参数：原绝对期限；返回：原对象全部清理后 true，到期前保留责任，原生失败返回原诊断。
    #[cfg(windows)]
    pub(crate) fn cleanup_until(&mut self, deadline: std::time::Instant) -> Result<bool, String> {
        if self.cleaned {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        let cleanup = self
            .windows_cleanup
            .as_mut()
            .ok_or("private Git bounded cleanup requires original native recovery payload")?;
        let done = cleanup
            .cleanup_until(self.capacity.as_ref(), deadline)
            .map_err(|error| format!("private Git cleanup failed: {error}"))?;
        if done {
            self.cleaned = true;
        }
        Ok(done)
    }

    /// 参数：无；返回：原根身份确认后的删除结果；失败保留原payload，绝不清理陌生替换根。
    pub(crate) fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned {
            return Ok(());
        }
        #[cfg(windows)]
        if let Some(cleanup) = self.windows_cleanup.as_mut() {
            cleanup
                .cleanup(self.capacity.as_ref())
                .map_err(|error| format!("private Git cleanup failed: {error}"))?;
            self.cleaned = true;
            return Ok(());
        }
        // 根路径不存在只确认名称消失；原对象可能已改名，不能释放其恢复责任。
        match std::fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    "private Git cleanup original root path missing; original object deletion unconfirmed"
                        .into(),
                );
            }
            Err(error) => return Err(format!("private Git cleanup root identity check: {error}")),
            Ok(_) => {}
        }
        let expected = self
            .root_identity
            .as_ref()
            .ok_or("private Git cleanup root identity unavailable")?;
        let current = GitPrivateAllocation::capture(&self.path)
            .map_err(|error| format!("private Git cleanup root identity: {error}"))?;
        if !expected.same_identity(&current) {
            return Err("private Git cleanup root identity changed; foreign root retained".into());
        }
        // 身份复核与 remove_dir_all 之间仍非原子；不声称隔离全部同权限竞态。
        match std::fs::remove_dir_all(&self.path) {
            Ok(()) => {
                self.cleaned = true;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // 缺名可能来自根或子项的移动竞态；再次查路径也不能确认原对象删除。
                Err(format!(
                    "private Git cleanup failed at {:?}: {error}; original object deletion unconfirmed",
                    self.path
                ))
            }
            Err(error) => Err(format!(
                "private Git cleanup failed at {:?}: {error}",
                self.path
            )),
        }
    }
}
