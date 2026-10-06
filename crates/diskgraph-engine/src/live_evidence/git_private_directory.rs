use super::git_directory_lease::GitDirectoryLease;
use super::git_private_allocation::GitPrivateAllocation;
use super::git_private_capacity::GitPrivateCapacity;
use super::git_private_directory_owner::GitPrivateDirectoryOwner;
use super::probe_budget::ProbeBudget;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 一次 Git 采样独占的临时目录。来源：原生 Rust Git 私有执行视图。
pub(super) struct GitPrivateDirectory {
    path: PathBuf,
    owner: Option<GitPrivateDirectoryOwner>,
    #[cfg(windows)]
    binding: Option<crate::probe_directory_binding::ProbeDirectoryBinding>,
}

impl GitPrivateDirectory {
    /// 以 128 MiB 对象报告分配和 64 MiB 卷可用余量建立目录。参数：probe 为整次期限及取消。返回：目录 owner 或明确拒绝。
    pub(super) fn new(probe: &mut ProbeBudget) -> Result<Self, String> {
        Self::with_limits(128 << 20, 64 << 20, probe)
    }

    /// 按实际对象报告分配及卷可用余量建立目录，不改变公开 ProbeLimits。
    /// 参数：quota 为对象分配上限，min_free 为卷保留余量，probe 为整次预算。返回：独占 owner 或明确拒绝。
    pub(super) fn with_limits(
        quota: u64,
        min_free: u64,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        {
            Self::with_windows_limits(quota, min_free, probe)
        }
        #[cfg(not(windows))]
        {
            probe.check().map_err(|error| error.to_string())?;
            // 只规范化受信 temp 根，不规范化待捕获的仓库元数据路径。
            let root = std::env::temp_dir()
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if !root.is_absolute() || !root.is_dir() {
                return Err("unsupported private directory root".into());
            }
            GitPrivateCapacity::check_volume(&root, 0, min_free, probe)?;
            for _ in 0..8 {
                probe.check().map_err(|error| error.to_string())?;
                let path = root.join(format!("diskgraph-git-{}", uuid::Uuid::new_v4()));
                #[cfg(unix)]
                let created = {
                    use std::os::unix::fs::DirBuilderExt;
                    std::fs::DirBuilder::new().mode(0o700).create(&path)
                };
                #[cfg(not(any(unix, windows)))]
                let created = Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "unsupported private directory platform",
                ));
                match created {
                    Ok(()) => {
                        // 创建后立即交给 owner；最终取消检查失败必须显式报告清理结果。
                        let mut directory = Self {
                            owner: Some(GitPrivateDirectoryOwner {
                                path: path.clone(),
                                cleaned: false,
                                capacity: None,
                                root_identity: None,
                            }),
                            path,
                        };
                        match GitPrivateAllocation::capture(&directory.path) {
                            Ok(identity) if identity.is_directory() => {
                                directory
                                    .owner
                                    .as_mut()
                                    .expect("new directory owner")
                                    .root_identity = Some(identity)
                            }
                            Ok(_) => {
                                return directory
                                    .complete(Err("private Git root is not a directory".into()));
                            }
                            Err(error) => return directory.complete(Err(error)),
                        }
                        match GitPrivateCapacity::new(&directory.path, quota, min_free, probe) {
                            Ok(capacity) => {
                                directory
                                    .owner
                                    .as_mut()
                                    .expect("new directory owner")
                                    .capacity = Some(capacity)
                            }
                            Err(error) => return directory.complete(Err(error)),
                        }
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
    }

    #[cfg(windows)]
    fn with_windows_limits(
        quota: u64,
        min_free: u64,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        use super::git_directory_security::GitDirectorySecurity;
        use super::windows_git_cleanup::WindowsGitCleanup;
        use super::windows_git_private_root::WindowsGitPrivateRoot;
        probe.check().map_err(|error| error.to_string())?;
        let mut binding = probe
            .directory_binding()
            .map_err(|error| error.to_string())?;
        let parent = std::env::temp_dir()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        GitPrivateCapacity::check_volume(&parent, 0, min_free, probe)?;
        let lease = GitDirectoryLease::open(&parent, probe)?;
        let security = GitDirectorySecurity::new()?;
        for _ in 0..8 {
            probe.check().map_err(|error| error.to_string())?;
            let name = format!("diskgraph-git-{}", uuid::Uuid::new_v4());
            let path = parent.join(&name);
            // 在原子创建调用之前安装外owner；有效返回句柄在查询/错误投影前进入原槽。
            let cleanup = WindowsGitCleanup::new(
                lease
                    .leaf_file()
                    .try_clone()
                    .map_err(|error| error.to_string())?,
                path.clone(),
            );
            let mut directory = Self {
                path: path.clone(),
                binding: binding.take(),
                owner: Some(GitPrivateDirectoryOwner {
                    path: path.clone(),
                    cleaned: false,
                    capacity: None,
                    root_identity: None,
                    windows_cleanup: Some(cleanup),
                }),
            };
            let owner = directory.owner.as_mut().expect("prepared directory owner");
            let cleanup = owner
                .windows_cleanup
                .as_mut()
                .expect("prepared native cleanup");
            let created = WindowsGitPrivateRoot::create_into(
                lease.leaf_file(),
                std::ffi::OsStr::new(&name),
                &security,
                &mut cleanup.root,
            );
            if let Err(error) = created {
                if cleanup.root.is_none() {
                    // 没有取得对象则不接管名字或未知对象；碰撞只重试独占创建。
                    directory.owner = None;
                    binding = directory.binding.take();
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(error.to_string());
                }
                return directory.complete(Err(error.to_string()));
            }
            let file = cleanup
                .root
                .as_ref()
                .expect("created original root")
                .as_file();
            let identity = match GitPrivateAllocation::from_file(file) {
                Ok(identity) => identity,
                Err(error) => return directory.complete(Err(error)),
            };
            owner.root_identity = Some(identity);
            let capacity = match GitPrivateCapacity::new(&path, quota, min_free, probe) {
                Ok(capacity) => capacity,
                Err(error) => return directory.complete(Err(error)),
            };
            if let Err(error) = capacity.check_identity(&path, file, true) {
                return directory.complete(Err(error));
            }
            owner.capacity = Some(capacity);
            if let Err(error) = probe.check() {
                return directory.complete(Err(error.to_string()));
            }
            return Ok(directory);
        }
        Err("private directory collision limit exceeded".into())
    }

    /// 取得私有目录路径。参数：无。返回：本次目录的借用路径。
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// 逐组件在已登记父句柄下建立私有目录，不收养既有陌生对象。
    /// 参数：path 为 owner 根内路径，probe 为整次预算。返回：目录建立且分配/卷余量门禁通过。
    pub(super) fn create_dir_all(
        &mut self,
        path: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        let capacity = self.active_capacity()?;
        capacity.validate_path(path)?;
        let mut current = self.path.clone();
        for component in path
            .strip_prefix(&self.path)
            .map_err(|_| "private Git root mismatch")?
            .components()
        {
            probe.check().map_err(|error| error.to_string())?;
            let std::path::Component::Normal(name) = component else {
                return Err("unsupported private Git directory component".into());
            };
            let parent = current.clone();
            current.push(name);
            let lease = GitDirectoryLease::open(&parent, probe)
                .map_err(|error| format!("private Git parent lease: {error}"))?;
            let capacity = self.active_capacity()?;
            capacity.check_identity(&parent, lease.leaf_file(), true)?;
            if capacity.registered(&current) {
                let child = GitDirectoryLease::open(&current, probe)
                    .map_err(|error| format!("private Git directory lease: {error}"))?;
                capacity.check_identity(&current, child.leaf_file(), true)?;
                capacity.finish_operation(probe)?;
                continue;
            }
            capacity.preflight(&current, 0, probe)?;
            let file = GitPrivateAllocation::create_directory(lease.leaf_file(), name)?;
            capacity.observe(&current, &file, probe)?;
            capacity.observe(&parent, lease.leaf_file(), probe)?;
            capacity.finish_operation(probe)?;
        }
        self.active_capacity()?.finish_operation(probe)
    }

    /// 独占创建文件或通过完整身份核对覆盖本 owner 已登记普通文件。
    /// 参数：path 为已登记父目录内目标，bytes 为完整内容，probe 为整次预算。返回：写入、实际分配及余量均确认。
    pub(super) fn write(
        &mut self,
        path: &Path,
        bytes: &[u8],
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        let parent = path.parent().ok_or("private Git file parent missing")?;
        let name = path.file_name().ok_or("private Git file name missing")?;
        self.active_capacity()?
            .preflight(path, bytes.len() as u64, probe)?;
        let lease = GitDirectoryLease::open(parent, probe)
            .map_err(|error| format!("private Git parent lease: {error}"))?;
        let capacity = self.active_capacity()?;
        capacity.check_identity(parent, lease.leaf_file(), true)?;
        let registered = capacity.registered(path);
        let mut file = GitPrivateAllocation::open_write(lease.leaf_file(), name, !registered)?;
        if registered {
            capacity.check_identity(path, &file, false)?;
        } else if GitPrivateAllocation::from_file(&file)?.is_directory() {
            return Err("private Git file creation returned a directory".into());
        }
        // 原生身份确认后才截断；未知路径只能 CREATE_EXCLUSIVE，目标竞态不会覆盖。
        probe.check().map_err(|error| error.to_string())?;
        file.set_len(0)
            .map_err(|error| format!("private Git truncate: {error}"))?;
        for chunk in bytes.chunks(4096) {
            probe.check().map_err(|error| error.to_string())?;
            file.write_all(chunk)
                .map_err(|error| format!("private Git write: {error}"))?;
        }
        file.flush()
            .map_err(|error| format!("private Git flush: {error}"))?;
        probe.check().map_err(|error| error.to_string())?;
        let file = GitPrivateAllocation::finish_write(lease.leaf_file(), name, file)?;
        probe.check().map_err(|error| error.to_string())?;
        capacity.observe(path, &file, probe)?;
        capacity.observe(parent, lease.leaf_file(), probe)?;
        capacity.finish_operation(probe)
    }

    /// 一次独占创建并流式写入完整捕获文件，所有块共享同一容量 owner。
    /// 参数：path 为私有新文件，length 为已准入长度，write 为同步写入闭包，probe 为原预算。
    /// 返回：闭包结果；源或目标失败保留 owner 供显式清理，不重建容量额度。
    pub(super) fn write_stream<T>(
        &mut self,
        path: &Path,
        length: u64,
        probe: &mut ProbeBudget,
        write: impl FnOnce(&mut std::fs::File, &mut ProbeBudget) -> Result<T, String>,
    ) -> Result<T, String> {
        let parent = path.parent().ok_or("private Git stream parent missing")?;
        let name = path.file_name().ok_or("private Git stream name missing")?;
        self.active_capacity()?.preflight(path, length, probe)?;
        let lease = GitDirectoryLease::open(parent, probe)?;
        let capacity = self.active_capacity()?;
        capacity.check_identity(parent, lease.leaf_file(), true)?;
        if capacity.registered(path) {
            return Err("private Git stream target already exists".into());
        }
        let mut file = GitPrivateAllocation::open_write(lease.leaf_file(), name, true)?;
        let result = write(&mut file, probe)?;
        file.flush()
            .map_err(|e| format!("private Git stream flush: {e}"))?;
        if file.metadata().map_err(|e| e.to_string())?.len() != length {
            return Err("private Git stream length mismatch".into());
        }
        let file = GitPrivateAllocation::finish_write(lease.leaf_file(), name, file)?;
        capacity.observe(path, &file, probe)?;
        capacity.observe(parent, lease.leaf_file(), probe)?;
        capacity.finish_operation(probe)?;
        Ok(result)
    }

    /// 使用已登记原生句柄设置私有 index 时间，保留 racy index 的比较语义。
    /// 参数：path 为已登记普通文件，modified 为源文件精确时间，probe 为整次预算。返回：精确回读一致且容量确认。
    pub(super) fn set_modified(
        &mut self,
        path: &Path,
        modified: std::time::SystemTime,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        let parent = path.parent().ok_or("private Git file parent missing")?;
        let name = path.file_name().ok_or("private Git file name missing")?;
        self.active_capacity()?.preflight(path, 0, probe)?;
        let lease = GitDirectoryLease::open(parent, probe)
            .map_err(|error| format!("private Git parent lease: {error}"))?;
        let capacity = self.active_capacity()?;
        capacity.check_identity(parent, lease.leaf_file(), true)?;
        let file = GitPrivateAllocation::open_write(lease.leaf_file(), name, false)?;
        capacity.check_identity(path, &file, false)?;
        probe.check().map_err(|error| error.to_string())?;
        file.set_times(std::fs::FileTimes::new().set_modified(modified))
            .map_err(|error| format!("private Git modified time: {error}"))?;
        probe.check().map_err(|error| error.to_string())?;
        let file = GitPrivateAllocation::finish_write(lease.leaf_file(), name, file)?;
        probe.check().map_err(|error| error.to_string())?;
        let observed = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map_err(|error| format!("private Git modified time read: {error}"))?;
        if observed != modified {
            return Err("unsupported private Git modified time precision".into());
        }
        capacity.observe(path, &file, probe)?;
        capacity.observe(parent, lease.leaf_file(), probe)?;
        capacity.finish_operation(probe)
    }

    /// 在私有视图完成准备及采样末段核对整个登记树。
    /// 参数：probe 为整次预算。返回：实际分配、完整对象集合及卷余量确认，否则拒绝结果。
    pub(super) fn verify_capacity(&mut self, probe: &mut ProbeBudget) -> Result<(), String> {
        self.active_capacity()?.verify(probe)
    }

    fn active_capacity(&mut self) -> Result<&mut GitPrivateCapacity, String> {
        let owner = self
            .owner
            .as_mut()
            .ok_or("private Git owner retained for recovery")?;
        if owner.cleaned {
            return Err("private Git owner already completed".into());
        }
        owner
            .capacity
            .as_mut()
            .ok_or_else(|| "private Git capacity is not initialized".into())
    }

    /// 显式结束本次私有视图，先确认根身份，再实际删除原目录。
    /// 参数：result 为采样成功值或原始失败；可在创建后、准备失败及公开终态调用。
    /// 返回：实际删除成功时保留原结果；失败或原名称缺失时拒绝原成功，并保留主/清理诊断。
    /// 原名称缺失不证明已移动 owner 被删除；身份复核和删除非原子，不提供同权限竞态隔离。
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
        #[cfg(windows)]
        if let Some(binding) = self.binding.as_mut() {
            return binding.complete(&mut self.owner);
        }
        self.owner
            .as_mut()
            .ok_or("private Git owner retained for recovery")?
            .cleanup()
    }
}

impl Drop for GitPrivateDirectory {
    fn drop(&mut self) {
        // Drop 仅兜底；公开成功/失败必须先经 complete 显式确认。
        let _ = self.cleanup();
    }
}
