use std::ffi::CStr;
use std::fs::File;
use std::os::fd::AsRawFd;

/// 仅观测原目录中的失败槽目录项，不跟随链接、不打开正文、不参与准入决策。
/// 参数：directory 为原持有目录，name 为协议固定槽名；返回：粗粒度类型和诊断 errno。
/// 这是失败后的瞬时观测，不能证明原 openat 时刻的目录项状态。
pub(crate) fn observe(directory: &File, name: &CStr) -> (&'static str, Option<i32>) {
    let mut entry = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            entry.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } < 0
    {
        let errno = std::io::Error::last_os_error().raw_os_error();
        return (
            if errno == Some(libc::ENOENT) {
                "missing"
            } else {
                "unknown"
            },
            errno,
        );
    }
    let entry = unsafe { entry.assume_init() };
    let kind = match entry.st_mode & libc::S_IFMT {
        libc::S_IFREG => "regular",
        libc::S_IFDIR => "directory",
        libc::S_IFLNK => "symlink",
        _ => "other",
    };
    (kind, None)
}

#[cfg(test)]
mod tests {
    use super::observe;
    use std::fs::File;

    #[test]
    fn missing_observation_does_not_create_the_slot() {
        let root = tempfile::tempdir().unwrap();
        let directory = File::open(root.path()).unwrap();
        assert_eq!(
            observe(&directory, c"slot"),
            ("missing", Some(libc::ENOENT))
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn observes_entry_types_without_following_a_dangling_link() {
        let root = tempfile::tempdir().unwrap();
        let directory = File::open(root.path()).unwrap();
        File::create(root.path().join("regular")).unwrap();
        std::fs::create_dir(root.path().join("directory")).unwrap();
        std::os::unix::fs::symlink("absent", root.path().join("link")).unwrap();
        assert_eq!(observe(&directory, c"regular"), ("regular", None));
        assert_eq!(observe(&directory, c"directory"), ("directory", None));
        assert_eq!(observe(&directory, c"link"), ("symlink", None));
        assert!(!root.path().join("absent").exists());
    }

    #[test]
    fn real_slot_failure_returns_original_open_error_and_keeps_entry() {
        use crate::TrustedLocalRecoveryDomain;
        use crate::recovery_slot::SlotError;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let slot = root.path().join("supervisor_0.slot");
        std::os::unix::fs::symlink("absent", &slot).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let domain =
            TrustedLocalRecoveryDomain::from_host(File::open(root.path()).unwrap(), deadline)
                .unwrap();
        let Err(SlotError::Io(error)) = domain.reserve(deadline) else {
            panic!("must preserve the original openat refusal");
        };
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
        assert!(std::fs::symlink_metadata(&slot).unwrap().is_symlink());
        assert!(!root.path().join("absent").exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
