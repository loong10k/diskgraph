//! 授权根内按需依赖闭包映射到唯一私有 owner；来源：D35 / EC-04。
use super::git_directory_version::GitDirectoryVersion;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_native_path;
use super::git_private_directory::GitPrivateDirectory;
use super::git_scope_boundary::GitScopeBoundary;
use super::git_source_file::GitSourceFile;
#[cfg(all(test, windows))]
use super::git_source_windows_phase::GitSourceWindowsPhase;
use super::probe_budget::ProbeBudget;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 持有授权源边界和捕获记录，不拥有第二个临时目录/容量预算；来源：原生 Rust GitWorktreeCapture。
pub(super) struct GitWorktreeCapture {
    boundary: GitScopeBoundary,
    target: PathBuf,
    directories: BTreeMap<PathBuf, (GitDirectoryVersion, Vec<OsString>)>,
    files: BTreeMap<PathBuf, GitSourceFile>,
}
impl GitWorktreeCapture {
    /// 参数：boundary 为持有注册根，private 为既有唯一 owner，budget/probe 为原累计预算。
    /// 返回：已捕获明确项目子树的闭包 owner；任何特殊源或超限拒绝，不搜索祖先仓库。
    pub(super) fn capture(
        boundary: GitScopeBoundary,
        private: &mut GitPrivateDirectory,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        let target = private.path().join("source");
        private.create_dir_all(&target, probe)?;
        let project = boundary.project().to_path_buf();
        let mut capture = Self {
            boundary,
            target,
            directories: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        capture.copy(&project, private, budget, probe)?;
        let names = &capture
            .directories
            .get(&project)
            .ok_or("Git project is not a directory")?
            .1;
        if !names.iter().any(|name| name == ".git") {
            return Err("scoped Git requires an explicit repository root".into());
        }
        Ok(capture)
    }
    /// 参数：无；返回：明确项目的私有工作树位置，绝不返回源路径。
    pub(super) fn project(&self) -> PathBuf {
        self.target.join(self.boundary.project())
    }
    /// 参数：base 为已捕获路径，value 为配置中的原生绝对/相对路径，private/budget/probe 为原 owner/预算。
    /// 返回：依赖在授权原根内被捕获后的私有映射；越界在源读取前拒绝。
    pub(super) fn resolve(
        &mut self,
        base: &Path,
        value: &Path,
        private: &mut GitPrivateDirectory,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<PathBuf, String> {
        let base = base
            .strip_prefix(&self.target)
            .map_err(|_| "scoped Git mapping base outside capture")?;
        let original = git_native_path::resolve(&self.boundary.original(base), value)?;
        let relative = self.boundary.relative(&original)?;
        self.copy(&relative, private, budget, probe)?;
        Ok(self.target.join(relative))
    }
    fn copy(
        &mut self,
        relative: &Path,
        private: &mut GitPrivateDirectory,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        #[cfg(all(test, windows))]
        let _phase = GitSourceWindowsPhase::new("captured_subtree_initial");
        let mut pending = vec![relative.to_path_buf()];
        while let Some(relative) = pending.pop() {
            budget.check(probe)?;
            // 工作树中的第二个 Git 根可能被 status 再次发现；其原 gitfile/config 不能成为子进程来源。
            // 顶层 .git 内部是当前仓库元数据，另由 index/ODB/config 的既有特殊能力门禁处理。
            if let Ok(worktree_path) = relative.strip_prefix(self.boundary.project()) {
                let components: Vec<_> = worktree_path.components().collect();
                let marker = |component: &std::path::Component<'_>| {
                    component
                        .as_os_str()
                        .to_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(".git"))
                };
                if components.first().is_some_and(|first| !marker(first))
                    && components.iter().skip(1).any(marker)
                {
                    return Err("unsupported nested Git repository in scoped worktree".into());
                }
            }
            if self.files.contains_key(&relative) || self.directories.contains_key(&relative) {
                continue;
            }
            let directory = if relative.as_os_str().is_empty() {
                true
            } else {
                let parent = self
                    .boundary
                    .directory(relative.parent().ok_or("invalid scoped Git parent")?, probe)?;
                parent.is_directory(relative.file_name().ok_or("invalid scoped Git name")?)?
            };
            let target = self.target.join(&relative);
            if directory {
                let source = self.boundary.directory(&relative, probe)?;
                let version = source.version()?;
                budget.charge_entry(probe)?;
                let names = source.names(budget, probe)?;
                if version != source.version()? {
                    return Err("scoped Git source directory changed during enumeration".into());
                }
                private.create_dir_all(&target, probe)?;
                for name in &names {
                    pending.push(relative.join(name));
                }
                self.directories.insert(relative, (version, names));
            } else {
                let parent = self
                    .boundary
                    .directory(relative.parent().ok_or("invalid scoped Git parent")?, probe)?;
                private
                    .create_dir_all(target.parent().ok_or("invalid scoped Git target")?, probe)?;
                let file = GitSourceFile::capture(
                    &parent,
                    relative.file_name().ok_or("invalid scoped Git name")?,
                    &target,
                    private,
                    budget,
                    probe,
                )?;
                self.files.insert(relative, file);
            }
        }
        Ok(())
    }
    /// 参数：budget/probe 为捕获时的原余额与期限；返回：所有原生版本、目录名单和正文指纹复核一致。
    /// 这是变化检测，不是原子快照；Git 命令是否隔离由私有路径绑定单独保证。
    pub(super) fn verify(
        &self,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        #[cfg(all(test, windows))]
        let _phase = GitSourceWindowsPhase::new("captured_subtree_verify");
        self.boundary.verify(probe)?;
        for (path, (version, names)) in &self.directories {
            let source = self.boundary.directory(path, probe)?;
            if &source.version()? != version {
                return Err("scoped Git source directory changed".into());
            }
            if &source.names(budget, probe)? != names || &source.version()? != version {
                return Err("scoped Git source directory entries changed".into());
            }
        }
        for (path, file) in &self.files {
            let parent = self
                .boundary
                .directory(path.parent().ok_or("invalid scoped Git parent")?, probe)?;
            file.verify(
                &parent,
                path.file_name().ok_or("invalid scoped Git name")?,
                budget,
                probe,
            )?;
        }
        self.boundary.verify(probe)?;
        budget.check(probe)
    }
}
