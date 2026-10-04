use super::git_directory_lease::GitDirectoryLease;
use super::git_metadata_budget::GitMetadataBudget;
use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Component, Path, PathBuf};

/// 私有 Git 对象的增量原生分配账本。来源：原生 Rust Git 视图容量门禁。
/// 默认额限定对象报告的分配，不涵盖文件系统全局元数据，也不是卷空间预留。
pub(super) struct GitPrivateCapacity {
    root: PathBuf,
    quota: u64,
    min_free: u64,
    used: u64,
    allocations: BTreeMap<PathBuf, GitPrivateAllocation>,
}

impl GitPrivateCapacity {
    /// 捕获新 owner 的根目录并建立账本。参数：root 为独占目录，quota/min_free 为分配及卷余量限制，probe 为整次预算。
    /// 返回：已计入根分配的账本；任何未知能力、超限或中止拒绝。
    pub(super) fn new(
        root: &Path,
        quota: u64,
        min_free: u64,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        Self::check_volume(root, 0, min_free, probe)?;
        let lease = GitDirectoryLease::open(root, probe)?;
        let allocation = GitPrivateAllocation::from_file(lease.leaf_file())?;
        if !allocation.is_directory() || allocation.bytes() > quota {
            probe.mark_resource_limit();
            return Err("private Git allocation quota exceeded by root directory".into());
        }
        let used = allocation.bytes();
        let mut allocations = BTreeMap::new();
        allocations.insert(root.to_owned(), allocation);
        probe.check().map_err(|error| error.to_string())?;
        Ok(Self {
            root: root.to_owned(),
            quota,
            min_free,
            used,
            allocations,
        })
    }

    /// 原生查询所在卷可用量，不把失败当无限空间。参数：path 为实际存在对象，required 为保守预留的逻辑写入量，min_free 为余量，probe 为整次预算。
    /// 返回：可用量覆盖 required + min_free 时成功；不是预留或竞态后的硬保证。
    pub(super) fn check_volume(
        path: &Path,
        required: u64,
        min_free: u64,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        probe.check().map_err(|error| error.to_string())?;
        let space = diskgraph_disktree_core::space::space_info(path)
            .map_err(|error| format!("private Git native space query: {error}"))?;
        let needed = required.checked_add(min_free).ok_or_else(|| {
            probe.mark_resource_limit();
            "private Git space requirement overflow"
        })?;
        if space.total == 0 || space.available > space.free || space.free > space.total {
            return Err("unsupported private Git native space accounting".into());
        }
        if space.available < needed {
            probe.mark_resource_limit();
            return Err(format!(
                "private Git available space below required headroom: available={} required={needed}",
                space.available
            ));
        }
        probe.check().map_err(|error| error.to_string())
    }

    /// 校验词法根边界。参数：path 为绝对私有路径。返回：只含正常相对组件的路径或拒绝。
    pub(super) fn validate_path(&self, path: &Path) -> Result<(), String> {
        if !path.is_absolute() || !path.starts_with(&self.root) {
            return Err("private Git path outside owner root".into());
        }
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| "private Git path outside owner root")?;
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err("unsupported private Git path component".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let raw = path.as_os_str().as_bytes();
            if raw.len() > 32768
                || raw.contains(&0)
                || raw
                    .split(|byte| *byte == b'/')
                    .any(|part| part == b"." || part == b"..")
            {
                return Err("unsupported private Git path encoding".into());
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let raw: Vec<u16> = relative.as_os_str().encode_wide().take(32769).collect();
            if raw.len() > 32768
                || raw.iter().any(|unit| matches!(*unit, 0 | 58))
                || relative.components().any(|component| match component {
                    Component::Normal(name) => {
                        let wide: Vec<u16> = name.encode_wide().collect();
                        wide == [46]
                            || wide == [46, 46]
                            || wide.last().is_some_and(|unit| matches!(*unit, 32 | 46))
                    }
                    _ => true,
                })
            {
                return Err("unsupported private Git path encoding".into());
            }
        }
        Ok(())
    }

    /// 判断路径是否由本 owner 登记。参数：path 为受控路径。返回：存在已登记对象时 true。
    pub(super) fn registered(&self, path: &Path) -> bool {
        self.allocations.contains_key(path)
    }

    /// 核对操作句柄与账本完整身份。参数：path 为登记路径，file 为 nofollow 持有句柄，directory 为要求类型。
    /// 返回：身份、类型一致时成功；不收养陌生路径或替换对象。
    pub(super) fn check_identity(
        &self,
        path: &Path,
        file: &File,
        directory: bool,
    ) -> Result<(), String> {
        self.validate_path(path)?;
        let expected = self
            .allocations
            .get(path)
            .ok_or("private Git object is not registered")?;
        let current = GitPrivateAllocation::from_file(file)?;
        if expected.is_directory() != directory || !expected.same_identity(&current) {
            return Err("private Git registered identity changed".into());
        }
        if !directory && !expected.same_version(&current) {
            return Err("private Git registered file version changed".into());
        }
        Ok(())
    }

    /// 每次操作前按逻辑长度作保守准入并检查卷余量。参数：path 为目标，logical_bytes 为本次完整内容长度，probe 为整次预算。
    /// 返回：允许继续尝试；逻辑长度可能大于压缩/驻留分配；实际分配必须在操作后再核算。
    pub(super) fn preflight(
        &self,
        path: &Path,
        logical_bytes: u64,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        self.validate_path(path)?;
        if !self.allocations.contains_key(path) && self.allocations.len() >= 32_768 {
            probe.mark_resource_limit();
            return Err("private Git allocation entry limit exceeded".into());
        }
        let old = self
            .allocations
            .get(path)
            .map_or(0, GitPrivateAllocation::bytes);
        let conservative_admission = self
            .used
            .checked_sub(old)
            .and_then(|used| used.checked_add(logical_bytes))
            .ok_or_else(|| {
                probe.mark_resource_limit();
                "private Git allocation accounting overflow"
            })?;
        if conservative_admission > self.quota {
            probe.mark_resource_limit();
            return Err(
                "private Git allocation quota exceeded by conservative apparent-length admission"
                    .into(),
            );
        }
        Self::check_volume(&self.root, logical_bytes, self.min_free, probe)
    }

    /// 用刚操作的句柄更新单项实际分配。参数：path 为目标，file 为操作句柄，probe 为整次预算。
    /// 返回：原身份和卷保持且更新后额度允许；每次更新 O(log n)，不遍历整树。
    pub(super) fn observe(
        &mut self,
        path: &Path,
        file: &File,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        probe.check().map_err(|error| error.to_string())?;
        self.validate_path(path)?;
        if !self.allocations.contains_key(path) && self.allocations.len() >= 32_768 {
            probe.mark_resource_limit();
            return Err("private Git allocation entry limit exceeded".into());
        }
        let current = GitPrivateAllocation::from_file(file)?;
        let root = self
            .allocations
            .get(&self.root)
            .ok_or("private Git root allocation missing")?;
        if !root.same_volume(&current) {
            return Err("private Git allocation volume changed".into());
        }
        if let Some(old) = self.allocations.get(path)
            && !old.same_identity(&current)
        {
            return Err("private Git registered identity changed".into());
        }
        let old = self
            .allocations
            .get(path)
            .map_or(0, GitPrivateAllocation::bytes);
        let used = self
            .used
            .checked_sub(old)
            .and_then(|used| used.checked_add(current.bytes()))
            .ok_or_else(|| {
                probe.mark_resource_limit();
                "private Git allocation accounting overflow"
            })?;
        self.used = used;
        self.allocations.insert(path.to_owned(), current);
        if used > self.quota {
            probe.mark_resource_limit();
            return Err(format!(
                "private Git allocation quota exceeded: reported={used} quota={}",
                self.quota
            ));
        }
        probe.check().map_err(|error| error.to_string())
    }

    /// 操作末段复核卷实际余量。参数：probe 为共享期限/取消。返回：空间仍可确认时成功。
    pub(super) fn finish_operation(&self, probe: &mut ProbeBudget) -> Result<(), String> {
        Self::check_volume(&self.root, 0, self.min_free, probe)
    }

    /// 在发布给 Git 前单次全树枚举核对登记对象、实际分配和卷余量，避免每次写入重复整树扫描。
    /// 参数：probe 为共享期限/取消。返回：登记对象集合、普通文件原生完整指纹及容量确认；不是字节摘要或原子文件系统快照。
    pub(super) fn verify(&self, probe: &mut ProbeBudget) -> Result<(), String> {
        self.finish_operation(probe)?;
        let mut budget = GitMetadataBudget::default();
        let mut pending = vec![self.root.clone()];
        let mut visited = BTreeSet::new();
        let mut reported = 0u64;
        while let Some(path) = pending.pop() {
            let mut lease = GitDirectoryLease::open(&path, probe)?;
            let current = GitPrivateAllocation::from_file(lease.leaf_file())?;
            let expected = self
                .allocations
                .get(&path)
                .ok_or("private Git directory is not registered")?;
            if !current.is_directory() || !expected.same_identity(&current) {
                return Err("private Git registered directory identity changed".into());
            }
            self.add_verified_allocation(&mut reported, current.bytes(), probe)?;
            let version = lease.version()?;
            let names = lease.read_names(&path, &mut budget, probe)?;
            visited.insert(path.clone());
            for name in names {
                probe.check().map_err(|error| error.to_string())?;
                let child = path.join(name);
                let expected = self
                    .allocations
                    .get(&child)
                    .ok_or("private Git contains unregistered object")?;
                if expected.is_directory() {
                    pending.push(child);
                } else {
                    let file = GitPrivateAllocation::open_read(
                        lease.leaf_file(),
                        child.file_name().ok_or("private Git leaf missing")?,
                    )?;
                    // 被动终检仅捕获一次 current，并与 own write 保存的原水位比较，绝不收养观察到的变化。
                    let current = GitPrivateAllocation::from_file(&file)?;
                    if !expected.same_version(&current) {
                        return Err("private Git registered file version changed".into());
                    }
                    self.add_verified_allocation(&mut reported, current.bytes(), probe)?;
                    visited.insert(child);
                }
            }
            if version != lease.version()? {
                return Err("private Git directory changed during allocation verification".into());
            }
        }
        if visited.len() != self.allocations.len() {
            return Err("private Git registered object disappeared".into());
        }
        self.finish_operation(probe)
    }

    fn add_verified_allocation(
        &self,
        total: &mut u64,
        bytes: u64,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        *total = total.checked_add(bytes).ok_or_else(|| {
            probe.mark_resource_limit();
            "private Git allocation accounting overflow"
        })?;
        if *total > self.quota {
            probe.mark_resource_limit();
            return Err(format!(
                "private Git allocation quota exceeded: reported={total} quota={}",
                self.quota
            ));
        }
        Ok(())
    }
}
