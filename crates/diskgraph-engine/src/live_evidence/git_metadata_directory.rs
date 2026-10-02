use super::git_directory_lease::GitDirectoryLease;
use super::git_directory_version::GitDirectoryVersion;
use super::git_metadata_budget::GitMetadataBudget;
use super::probe_budget::ProbeBudget;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Git 元数据目录的不可变版本、祖先身份及有界名称记录；不保留描述符。
/// 来源：原生 Rust no-follow 目录捕获与短期租约；这是变化检测，不是原子快照。
pub(super) struct GitMetadataDirectory {
    path: PathBuf,
    names: Vec<OsString>,
    initial: GitDirectoryVersion,
    parents: Vec<GitDirectoryVersion>,
}

impl GitMetadataDirectory {
    /// 捕获普通目录，并在返回前释放全部文件与祖先句柄。
    /// 参数：path 为无链接的原生绝对路径，budget/probe 为共享额度、期限与取消。
    /// 返回：不可变目录版本和名称记录，或路径、资源、取消及变化错误。
    pub(super) fn capture(
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        budget.charge_entry(probe)?;
        let mut lease = GitDirectoryLease::open(path, probe)?;
        let initial = lease.version()?;
        let parents = lease.parent_versions(probe)?;
        let mut names = lease.read_names(path, budget, probe)?;
        names.sort();
        budget.check(probe)?;
        if lease.version()? != initial {
            return Err("git metadata directory version changed".into());
        }
        // 枚举时仍持有完整租约；记录完成后不随整棵视图累积 fd/HANDLE。
        drop(lease);
        budget.check(probe)?;
        Ok(Self {
            path: path.to_owned(),
            names,
            initial,
            parents,
        })
    }

    /// 取得捕获路径。参数：无。返回：没有 canonicalize 的原生绝对路径。
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// 取得叶名称列表。参数：无。返回：按原生名称排序并保留原编码的有界借用列表。
    pub(super) fn names(&self) -> &[OsString] {
        &self.names
    }

    /// 重新逐组件 no-follow 打开并比较完整版本、祖先身份及名单。
    /// 参数：budget/probe 为同一次采样累计预算。
    /// 返回：当前路径仍绑定原目录及原祖先且名称未变化，或明确错误；不提供跨文件原子快照。
    pub(super) fn verify(
        &self,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        budget.check(probe)?;
        let current = Self::capture(self.path(), budget, probe)?;
        if self.initial != current.initial
            || self.names != current.names
            || self.parents.len() != current.parents.len()
        {
            return Err("git metadata directory changed".into());
        }
        for (original, current) in self.parents.iter().zip(&current.parents) {
            budget.check(probe)?;
            if !original.same_identity(current) {
                return Err("git metadata directory parent changed".into());
            }
        }
        budget.check(probe)
    }
}
