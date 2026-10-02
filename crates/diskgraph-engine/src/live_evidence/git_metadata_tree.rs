//! 捕获需要的元数据路径和目录名单，缺失记录也由已捕获父目录证明。

use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_directory::GitMetadataDirectory;
use super::git_metadata_file::GitMetadataFile;
use super::probe_budget::ProbeBudget;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 普通元数据的初始记录集合；仅在成功末段复核后可发布样本。
/// 来源：原生 Rust DiskGraph Git 元数据变化检测，无 Java 对应实现。
#[derive(Default)]
pub(super) struct GitMetadataTree {
    directories: BTreeMap<PathBuf, GitMetadataDirectory>,
    files: Vec<(PathBuf, GitMetadataFile)>,
    file_indices: BTreeMap<PathBuf, usize>,
}

impl GitMetadataTree {
    /// 捕获存在目录；缺失时捕获最近存在的父目录名单证明缺失。
    /// 参数：path 为绝对原始路径；budget/probe 为整次元数据额度及执行预算。
    /// 返回：存在为 true；路径错误不作为缺失。
    pub(super) fn directory(
        &mut self,
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<bool, String> {
        budget.check(probe)?;
        if self.directories.contains_key(path) {
            return Ok(true);
        }
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                self.directories.insert(
                    path.to_owned(),
                    GitMetadataDirectory::capture(path, budget, probe)?,
                );
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = path.parent().ok_or("invalid Git metadata root")?;
                if self.directory(parent, budget, probe)? {
                    let directory = self.directories.get(parent).expect("captured parent");
                    if contains(directory.names(), path)? {
                        return Err("Git metadata directory changed during discovery".into());
                    }
                }
                Ok(false)
            }
            Ok(_) => Err("unsupported Git metadata directory type".into()),
            Err(error) => Err(format!("Git metadata directory: {error}")),
        }
    }

    /// 捕获可选文件；名单未命中时查证原生路径，存在别名或发现竞态必须拒绝。
    /// 参数：path 为原始绝对路径；budget/probe 为共享预算。
    /// 返回：文件记录位置，或由父目录证明的缺失。
    pub(super) fn file(
        &mut self,
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Option<usize>, String> {
        if let Some(index) = self.file_indices.get(path) {
            return Ok(Some(*index));
        }
        let parent = path.parent().ok_or("invalid Git metadata file parent")?;
        if !self.directory(parent, budget, probe)? {
            return Ok(None);
        }
        let directory = self.directories.get(parent).expect("captured parent");
        if !contains(directory.names(), path)? {
            return Ok(None);
        }
        self.detached_file(path, budget, probe).map(Some)
    }

    /// 捕获 gitfile 自身，避免把实时工作树根的目录版本当成元数据快照。
    /// 参数：path 为已发现存在的普通文件；budget/probe 为共享预算。
    /// 返回：记录位置；发现后消失返回错误。
    pub(super) fn detached_file(
        &mut self,
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<usize, String> {
        let file = GitMetadataFile::capture(path, budget, probe)?;
        if file.bytes().is_none() {
            return Err("Git metadata disappeared during discovery".into());
        }
        self.files.push((path.to_owned(), file));
        self.file_indices
            .insert(path.to_owned(), self.files.len() - 1);
        Ok(self.files.len() - 1)
    }

    /// 借用已捕获文件。参数：index 为本集合返回的位置。返回：普通文件记录。
    pub(super) fn get(&self, index: usize) -> &GitMetadataFile {
        &self.files[index].1
    }

    /// 借用已捕获目录的原生名称，避免另一次无预算枚举。
    /// 参数：path 为已捕获目录。返回：排序名称；未捕获返回空名单。
    pub(super) fn names(&self, path: &Path) -> &[OsString] {
        self.directories
            .get(path)
            .map_or(&[], |directory| directory.names())
    }

    /// 复制所需 loose refs，不复制源 commondir/对象库/配置能力。
    /// 参数：source/target 为原目录与私有目标；budget/probe 为共享预算。
    /// 返回：完整复制，或特殊路径/预算错误。
    pub(super) fn copy_refs(
        &mut self,
        source: &Path,
        target: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        let mut stack = vec![(source.to_owned(), target.to_owned())];
        while let Some((source, target)) = stack.pop() {
            if !self.directory(&source, budget, probe)? {
                continue;
            }
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("private Git refs: {error}"))?;
            let names = self.names(&source).to_vec();
            for name in names {
                budget.check(probe)?;
                let path = source.join(&name);
                let kind = std::fs::symlink_metadata(&path)
                    .map_err(|error| format!("Git ref discovery: {error}"))?;
                if kind.file_type().is_symlink() {
                    return Err("unsupported Git ref link".into());
                }
                if kind.is_dir() {
                    stack.push((path, target.join(name)));
                } else {
                    let index = self
                        .file(&path, budget, probe)?
                        .ok_or("Git ref disappeared")?;
                    std::fs::write(
                        target.join(name),
                        self.get(index).bytes().expect("captured file"),
                    )
                    .map_err(|error| format!("private Git ref: {error}"))?;
                }
            }
        }
        Ok(())
    }

    /// 复核所有来源目录及文件；失败不得返回完整样本。
    /// 参数：budget/probe 继续使用同一原始输入额度、期限和取消。
    /// 返回：身份、版本、名单及字节均稳定；这是变化检测，不是原子仓库快照。
    pub(super) fn verify(
        &self,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        for directory in self.directories.values() {
            directory.verify(budget, probe)?;
        }
        for (_, file) in &self.files {
            file.verify(budget, probe)?;
        }
        budget.check(probe)
    }
}

fn contains(names: &[OsString], path: &Path) -> Result<bool, String> {
    let name = path.file_name().ok_or("invalid Git metadata leaf")?;
    if names
        .binary_search_by(|candidate| candidate.as_os_str().cmp(name))
        .is_ok()
    {
        return Ok(true);
    }
    // 原生卷可能不区分大小写或采用其他别名规则；精确名单缺项不能证明路径缺失。
    // 仅查询叶身份、不读取内容；不能可靠匹配时拒绝，避免丢弃 config/index 后伪造状态。
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err("unsupported Git metadata name alias or changed directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Git metadata absence check: {error}")),
    }
}
