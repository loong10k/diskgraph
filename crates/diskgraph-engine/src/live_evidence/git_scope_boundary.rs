//! 固定注册根与无损项目定位的词法边界；来源：D35 / EC-04，不执行身份授权。
use super::git_directory_version::GitDirectoryVersion;
use super::git_source_directory::GitSourceDirectory;
use super::probe_budget::ProbeBudget;
use diskgraph_core::QualifiedLocator;
use std::path::{Component, Path, PathBuf};

/// 持有唯一授权根能力，所有项目和依赖都相对于此根打开；来源：原生 Rust scoped Git。
pub(super) struct GitScopeBoundary {
    root: GitSourceDirectory,
    root_version: GitDirectoryVersion,
    root_route: Vec<GitDirectoryVersion>,
    root_path: PathBuf,
    project: PathBuf,
}
impl GitScopeBoundary {
    /// 参数：root 为已授权注册根，project 为同范围持久无损定位，probe 为同一期限；返回：持有边界或拒绝。
    pub(super) fn new(
        root: &Path,
        project: &QualifiedLocator,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        let path = project.to_native_path().map_err(|e| e.to_string())?;
        let relative = relative(root, &path)?;
        // 初次注册根/项目打开也处于线程级不物化策略内，不能等到复制阶段才进入。
        let _hydration =
            crate::scoped_content::ScopedContent::hydration_guard().map_err(|e| e.to_string())?;
        let (root_file, root_route) = GitSourceDirectory::root(root, probe)?;
        let root_version = root_file.version()?;
        let boundary = Self {
            root: root_file,
            root_version,
            root_route,
            root_path: root.to_path_buf(),
            project: relative,
        };
        boundary.directory(&boundary.project, probe)?;
        Ok(boundary)
    }
    /// 参数：probe 为原共享期限；返回：原路由各组件身份及持有注册根版本未变时成功。
    pub(super) fn verify(&self, probe: &mut ProbeBudget) -> Result<(), String> {
        // 仅重走原根路由核验身份，不从新路径读取项目正文；祖先旁支活动不构成版本冲突。
        let (_, route) = GitSourceDirectory::root(&self.root_path, probe)?;
        if route.len() != self.root_route.len()
            || self
                .root_route
                .iter()
                .zip(&route)
                .any(|(old, now)| !old.same_identity(now))
        {
            return Err("scoped Git registered root route changed".into());
        }
        if self.root.version()? != self.root_version {
            return Err("scoped Git registered root changed".into());
        }
        Ok(())
    }
    /// 参数：无；返回：根下精确项目相对路径，不进行祖先发现。
    pub(super) fn project(&self) -> &Path {
        &self.project
    }
    /// 参数：path 为原绝对依赖；返回：经词法范围核验的相对定位，不读取来源。
    pub(super) fn relative(&self, path: &Path) -> Result<PathBuf, String> {
        relative(&self.root_path, path)
    }
    /// 参数：relative 为根内路径；返回：仅用于解析元数据路径的原定位，绝不交给 I/O。
    pub(super) fn original(&self, relative: &Path) -> PathBuf {
        self.root_path.join(relative)
    }
    /// 参数：relative 为根内正常组件，probe 为共享预算；返回：从同一 held root 逐组件打开的目录。
    pub(super) fn directory(
        &self,
        relative: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<GitSourceDirectory, String> {
        let mut directory = self.root.duplicate()?;
        if relative.components().count() > 1024 {
            return Err("scoped Git directory depth exceeded".into());
        }
        for component in relative.components() {
            probe.check().map_err(|e| e.to_string())?;
            let Component::Normal(name) = component else {
                return Err("unsupported scoped Git relative component".into());
            };
            directory = directory.child(name)?;
        }
        Ok(directory)
    }
}
fn relative(root: &Path, path: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        return crate::windows_path_plan::WindowsPathPlan::relative_to_root(root, path)
            .map(|parts| parts.into_iter().collect())
            .map_err(|e| e.to_string());
    }
    #[cfg(not(windows))]
    {
        if !root.is_absolute()
            || !path.is_absolute()
            || root
                .components()
                .chain(path.components())
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("unsupported scoped Git path".into());
        }
        path.strip_prefix(root)
            .map(Path::to_path_buf)
            .map_err(|_| "Git source is outside authorized scope".into())
    }
}
