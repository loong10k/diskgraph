use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_version::GitMetadataVersion;
use super::probe_budget::ProbeBudget;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Git 原生元数据普通文件的有界内容和初始句柄版本；来源：Unix openat 或 Windows ScopedFile。
/// 捕获与末段复核重新解析精确路径；目录树另由目录捕获对象复核。
pub(super) struct GitMetadataFile {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    version: Option<GitMetadataVersion>,
}

impl GitMetadataFile {
    /// 捕获单个普通元数据文件，逐组件禁止链接且按元数据预算读取。
    /// 参数：path 为绝对路径，budget 为捕获及复核共享额度，probe 为期限和取消。
    /// 返回：原始字节/版本；只有明确缺失的末段记录为 None，其他路径错误拒绝。
    pub(super) fn capture(
        path: &Path,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        budget.charge_entry(probe)?;
        #[cfg(unix)]
        let captured = capture_unix(path, budget, probe)?;
        #[cfg(windows)]
        let captured = capture_windows(path, budget, probe)?;
        #[cfg(not(any(unix, windows)))]
        let captured: Option<(Vec<u8>, GitMetadataVersion)> = {
            let _ = (path, budget, probe);
            return Err("unsupported git metadata platform".into());
        };
        let (bytes, version) = match captured {
            Some((bytes, version)) => (Some(bytes), Some(version)),
            None => (None, None),
        };
        Ok(Self {
            path: path.to_path_buf(),
            bytes,
            version,
        })
    }

    /// 借用捕获的原始字节。参数：无。返回：存在文件时的完整原始字节；明确缺失时 None。
    pub(super) fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// 取得 index 私有副本需要保留的高精度 mtime。参数：无。返回：原生修改时间；缺失时 None。
    pub(super) fn modified(&self) -> Option<SystemTime> {
        self.version.as_ref().map(GitMetadataVersion::modified)
    }

    /// 重新打开路径、读取内容并与初始身份版本和字节比较。
    /// 参数：budget 为同一个原始输入额度，probe 为同一次期限和取消。
    /// 返回：完全一致时成功；替换、改写、预算耗尽或无法安全证明时错误。
    pub(super) fn verify(
        &self,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        let current = Self::capture(&self.path, budget, probe)?;
        match (&self.bytes, &self.version, current.bytes, current.version) {
            (None, None, None, None) => Ok(()),
            (Some(old), Some(version), Some(new), Some(current_version)) => {
                if old != &new || version.modified() != current_version.modified() {
                    return Err("git metadata changed during capture".into());
                }
                // 两轮各自已检查句柄版本；再次重开的身份由原始版本比较。
                #[cfg(unix)]
                if !version.same_initial(&current_version) {
                    return Err("git metadata identity changed".into());
                }
                #[cfg(windows)]
                if !version.same_initial(&current_version) {
                    return Err("git metadata identity changed".into());
                }
                Ok(())
            }
            _ => Err("git metadata presence changed".into()),
        }
    }
}

fn read_chunks(
    file: &mut File,
    len: u64,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<Vec<u8>, String> {
    if len > budget.remaining_bytes() as u64 || len > usize::MAX as u64 {
        return Err("git metadata byte limit exceeded".into());
    }
    let mut bytes = Vec::with_capacity(len as usize);
    let mut chunk = [0u8; 4096];
    loop {
        budget.check(probe)?;
        let size = file
            .read(&mut chunk)
            .map_err(|error| format!("git metadata read: {error}"))?;
        if size == 0 {
            break;
        }
        budget.charge_bytes(size, probe)?;
        if bytes
            .len()
            .checked_add(size)
            .is_none_or(|observed| observed as u64 > len)
        {
            return Err("git metadata length changed during read".into());
        }
        bytes.extend_from_slice(&chunk[..size]);
    }
    budget.check(probe)?;
    if bytes.len() as u64 != len {
        return Err("git metadata length changed during read".into());
    }
    Ok(bytes)
}

#[cfg(unix)]
fn capture_unix(
    path: &Path,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<Option<(Vec<u8>, GitMetadataVersion)>, String> {
    let Some(mut file) = open_unix(path, budget, probe)? else {
        return Ok(None);
    };
    let before = file
        .metadata()
        .map_err(|error| format!("git metadata stat: {error}"))?;
    let version = GitMetadataVersion::from_metadata(&before)?;
    let bytes = read_chunks(&mut file, before.len(), budget, probe)?;
    let after = file
        .metadata()
        .map_err(|error| format!("git metadata stat: {error}"))?;
    if !version.matches_metadata(&after) {
        return Err("git metadata changed during read".into());
    }
    Ok(Some((bytes, version)))
}

#[cfg(unix)]
fn open_unix(
    path: &Path,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<Option<File>, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    if !path.is_absolute() {
        return Err("git metadata path is not absolute".into());
    }
    let mut parent = File::open("/").map_err(|error| format!("git metadata root: {error}"))?;
    let mut components = Vec::new();
    for component in path.components() {
        budget.check(probe)?;
        match component {
            Component::RootDir => {}
            Component::Normal(name) => components.push(name),
            _ => return Err("invalid git metadata path component".into()),
        }
    }
    let leaf = components.pop().ok_or("invalid git metadata leaf")?;
    for name in components {
        budget.check(probe)?;
        let name = CString::new(name.as_bytes()).map_err(|_| "invalid git metadata path")?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "git metadata parent: {}",
                std::io::Error::last_os_error()
            ));
        }
        // 原生目录句柄逐层替换，拒绝任何父级链接。
        parent = unsafe { File::from_raw_fd(fd) };
    }
    budget.check(probe)?;
    let leaf = CString::new(leaf.as_bytes()).map_err(|_| "invalid git metadata path")?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(None)
        } else {
            Err(format!("git metadata leaf: {error}"))
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file
        .metadata()
        .map_err(|error| format!("git metadata stat: {error}"))?
        .is_file()
    {
        return Err("git metadata is not a regular file".into());
    }
    Ok(Some(file))
}

#[cfg(windows)]
fn capture_windows(
    path: &Path,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<Option<(Vec<u8>, GitMetadataVersion)>, String> {
    use crate::windows_scoped_file::WindowsScopedFile;
    use std::path::Component;
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Prefix(_)))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err("unsupported git metadata root".into());
    }
    for _ in path.components() {
        budget.check(probe)?;
    }
    let root: PathBuf = path.components().take(2).collect();
    // 现有租约 API 无法区分 parent 与 leaf 的 NotFound；不能把不明缺失当作安全的 None。
    let lease = WindowsScopedFile::open(&root, path)
        .map_err(|error| format!("open git metadata (missing path unsupported): {error}"))?;
    let metadata = lease
        .metadata()
        .map_err(|error| format!("git metadata stat: {error}"))?;
    let version = GitMetadataVersion::from_windows_state(
        &lease.state,
        metadata
            .modified()
            .map_err(|error| format!("git metadata modified time: {error}"))?,
    );
    let mut file = lease
        .open_data()
        .map_err(|error| format!("git metadata data: {error}"))?;
    let bytes = read_chunks(&mut file, lease.state.len, budget, probe)?;
    if !lease.matches(&file) {
        return Err("git metadata changed during read".into());
    }
    Ok(Some((bytes, version)))
}
