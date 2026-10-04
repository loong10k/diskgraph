//! 持有源目录的能力对象；来源：DiskGraph D35 scoped Git 捕获。
use super::git_directory_version::GitDirectoryVersion;
use super::git_metadata_budget::GitMetadataBudget;
#[cfg(unix)]
use super::git_source_unix as native;
#[cfg(windows)]
use super::git_source_windows as native;
use super::probe_budget::ProbeBudget;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::Path;

/// 一个不可回退为路径枚举的源目录句柄；来源：Unix openat 与 Windows NT RootDirectory。
pub(super) struct GitSourceDirectory {
    file: File,
}
impl GitSourceDirectory {
    /// 参数：root 是实际注册的原生根，probe 为共享期限；返回：不跟随链接的持有根及逐组件身份链。
    pub(super) fn root(
        root: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<(Self, Vec<GitDirectoryVersion>), String> {
        let (file, route) = native::root(root, probe)?;
        Ok((Self { file }, route))
    }
    /// 参数：name 为单个名称；返回：相对当前能力取得的新目录能力。
    pub(super) fn child(&self, name: &OsStr) -> Result<Self, String> {
        Ok(Self {
            file: native::child(&self.file, name, true)?,
        })
    }
    /// 参数：无；返回：同一已持有目录的独立 Rust 句柄，不重新解析原路径。
    pub(super) fn duplicate(&self) -> Result<Self, String> {
        Ok(Self {
            file: self.file.try_clone().map_err(|e| e.to_string())?,
        })
    }
    /// 参数：name 为单个名称；返回：普通目录为 true，普通文件为 false，特殊类型拒绝。
    pub(super) fn is_directory(&self, name: &OsStr) -> Result<bool, String> {
        native::is_directory(&self.file, name)
    }
    /// 参数：name 为普通文件名；返回：先属性核验再申请正文权能的文件句柄。
    pub(super) fn open_file(&self, name: &OsStr) -> Result<File, String> {
        native::child(&self.file, name, false)
    }
    /// 参数：budget/probe 为整次输入和执行预算；返回：来自同一目录句柄的完整有界名单。
    pub(super) fn names(
        &self,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Vec<OsString>, String> {
        native::names(&self.file, budget, probe)
    }
    /// 参数：无；返回：完整身份及不含 atime 的目录版本。
    pub(super) fn version(&self) -> Result<GitDirectoryVersion, String> {
        GitDirectoryVersion::capture(&self.file)
    }
}
