//! Unix 源访问只相对持有目录；来源：openat/fstatat/fdopendir，不解析源绝对路径。
use super::git_directory_version::GitDirectoryVersion;
use super::git_metadata_budget::GitMetadataBudget;
use super::probe_budget::ProbeBudget;
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path};

/// 参数：path 为注册绝对根，probe 为原期限；返回：逐组件 no-follow 的根目录句柄及从文件系统根开始的身份链。
pub(super) fn root(
    path: &Path,
    probe: &mut ProbeBudget,
) -> Result<(File, Vec<GitDirectoryVersion>), String> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || bytes.len() > 32768
        || path.components().count() > 1025
        || bytes.contains(&0)
        || bytes.split(|b| *b == b'/').any(|p| p == b"." || p == b"..")
    {
        return Err("unsupported scoped Git root".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|e| e.to_string())?;
    let mut route = vec![GitDirectoryVersion::capture(&file)?];
    for component in path.components() {
        probe.check().map_err(|e| e.to_string())?;
        if let Component::Normal(name) = component {
            file = child(&file, name, true)?;
            route.push(GitDirectoryVersion::capture(&file)?);
        }
    }
    Ok((file, route))
}

/// 参数：parent/name 为持有父句柄及单个原生名称，directory 为类型；返回：拒绝链接/特殊/占位后的只读句柄。
pub(super) fn child(parent: &File, name: &OsStr, directory: bool) -> Result<File, String> {
    let raw = name.as_bytes();
    if raw.is_empty() || raw == b"." || raw == b".." || raw.contains(&b'/') {
        return Err("invalid scoped Git component".into());
    }
    let name = CString::new(raw).map_err(|e| e.to_string())?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let stat = unsafe { stat.assume_init() };
    let expected = if directory {
        libc::S_IFDIR
    } else {
        libc::S_IFREG
    };
    if stat.st_mode & libc::S_IFMT != expected {
        return Err("unsupported scoped Git source type".into());
    }
    #[cfg(target_os = "macos")]
    if stat.st_flags & 0x40000000 != 0 {
        return Err("unsupported scoped Git dataless source".into());
    }
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    use std::os::unix::fs::MetadataExt;
    let now = file.metadata().map_err(|e| e.to_string())?;
    // Apple dev_t 为有符号值，保持原转换的符号扩展；Linux/Android 使用无损宽整数比较。
    #[cfg(target_vendor = "apple")]
    let same_device = now.dev() == stat.st_dev as u64;
    #[cfg(not(target_vendor = "apple"))]
    let same_device = i128::from(now.dev()) == i128::from(stat.st_dev);
    if !same_device
        || i128::from(now.ino()) != i128::from(stat.st_ino)
        || now.is_dir() != directory
        || (!directory && !now.is_file())
    {
        return Err("scoped Git source changed before open".into());
    }
    Ok(file)
}

/// 参数：parent/name 为源父目录与单名称；返回：不打开正文的普通目录分类，链接/特殊类型拒绝。
pub(super) fn is_directory(parent: &File, name: &OsStr) -> Result<bool, String> {
    let name = CString::new(name.as_bytes()).map_err(|e| e.to_string())?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let stat = unsafe { stat.assume_init() };
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => Ok(true),
        libc::S_IFREG => Ok(false),
        _ => Err("unsupported scoped Git source link or special file".into()),
    }
}

/// 参数：file 为持有目录，budget/probe 为共享预算；返回：同句柄枚举的有界排序名称。
pub(super) fn names(
    file: &File,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
) -> Result<Vec<OsString>, String> {
    // 独立 openat(".") 创建新的枚举偏移；dup 会与原句柄共享 offset。
    let fd = unsafe {
        libc::openat(
            file.as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let duplicate = unsafe { File::from_raw_fd(fd) };
    let stream = unsafe { libc::fdopendir(duplicate.as_raw_fd()) };
    if stream.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let _ = duplicate.into_raw_fd();
    let result = (|| {
        let mut names = Vec::new();
        loop {
            budget.check(probe)?;
            #[cfg(any(target_os = "linux", target_os = "android"))]
            unsafe {
                *libc::__errno_location() = 0;
            }
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            unsafe {
                *libc::__error() = 0;
            }
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error.to_string());
                }
                break;
            }
            let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            budget.charge_entry(probe)?;
            budget.charge_bytes(name.len(), probe)?;
            names.push(OsString::from_vec(name.to_vec()));
        }
        names.sort();
        Ok(names)
    })();
    let closed = unsafe { libc::closedir(stream) };
    if closed != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    result
}
