use super::probe_budget::ProbeBudget;
use std::path::{Path, PathBuf};

/// 一次 Git 采样独占的临时目录。来源：原生 Rust Git 私有执行视图。
pub(super) struct GitPrivateDirectory {
    path: PathBuf,
    cleaned: bool,
}

impl GitPrivateDirectory {
    /// 建立本次采样目录。参数：probe 为整次期限及取消。返回：目录所有权或创建错误。
    pub(super) fn new(probe: &mut ProbeBudget) -> Result<Self, String> {
        probe.check().map_err(|error| error.to_string())?;
        // 只规范化受信 temp 根，不规范化待捕获的仓库元数据路径。
        let root = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !root.is_absolute() || !root.is_dir() {
            return Err("unsupported private directory root".into());
        }
        #[cfg(windows)]
        let security = super::git_directory_security::GitDirectorySecurity::new()?;
        for _ in 0..8 {
            probe.check().map_err(|error| error.to_string())?;
            let path = root.join(format!("diskgraph-git-{}", uuid::Uuid::new_v4()));
            #[cfg(unix)]
            let created = {
                use std::os::unix::fs::DirBuilderExt;
                std::fs::DirBuilder::new().mode(0o700).create(&path)
            };
            #[cfg(windows)]
            let created = security.create(&path);
            #[cfg(not(any(unix, windows)))]
            let created = Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unsupported private directory platform",
            ));
            match created {
                Ok(()) => {
                    // 创建后立即交给 owner；最终取消检查失败必须显式报告清理结果。
                    let mut directory = Self {
                        path,
                        cleaned: false,
                    };
                    if let Err(error) = probe.check() {
                        return Err(directory
                            .complete::<()>(Err(error.to_string()))
                            .unwrap_err());
                    }
                    return Ok(directory);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("private directory collision limit exceeded".into())
    }

    /// 取得私有目录路径。参数：无。返回：本次目录的借用路径。
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// 显式结束本次私有视图，必须确认本 owner 的目录已删除。
    /// 参数：result 为采样成功值或原始失败；可在创建后、准备失败及公开终态调用。
    /// 返回：清理成功时保留原结果；失败时拒绝原成功，或在原错误后附加清理错误和受控路径。
    pub(super) fn complete<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        let cleanup = self.cleanup();
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(primary), Err(cleanup)) => {
                Err(format!("{primary}; cleanup also failed: {cleanup}"))
            }
        }
    }

    fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned {
            return Ok(());
        }
        match std::fs::remove_dir_all(&self.path) {
            Ok(()) => {
                self.cleaned = true;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // remove_dir_all 的 NotFound 也可能来自子项竞态；只确认 owner 根消失才成功。
                match std::fs::symlink_metadata(&self.path) {
                    Err(root_error) if root_error.kind() == std::io::ErrorKind::NotFound => {
                        self.cleaned = true;
                        Ok(())
                    }
                    Ok(_) => Err(format!(
                        "private Git cleanup failed at {:?}: {error}; owner root still exists",
                        self.path
                    )),
                    Err(root_error) => Err(format!(
                        "private Git cleanup failed at {:?}: {error}; owner root check failed: {root_error}",
                        self.path
                    )),
                }
            }
            Err(error) => Err(format!(
                "private Git cleanup failed at {:?}: {error}",
                self.path
            )),
        }
    }
}

impl Drop for GitPrivateDirectory {
    fn drop(&mut self) {
        // Drop 仅兜底；公开成功/失败必须先经 complete 显式确认。
        let _ = self.cleanup();
    }
}
