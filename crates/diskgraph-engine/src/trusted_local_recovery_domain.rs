use crate::recovery_slot::{SlotError, SlotReservation};
use std::fs::File;
#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

/// 可信本地 launcher 提供的用户持久恢复容量域；不认证任意同 UID 攻击者。
/// 来源：DiskGraph 原生 Rust PF-06 固定用户容量域，无 Java 对等对象。
/// 目录定位与全入口统一由 launcher 保证，本对象只在原 held 目录中认领槽。
pub struct TrustedLocalRecoveryDomain {
    directory: File,
}

impl TrustedLocalRecoveryDomain {
    /// 从 OS 当前用户记录打开统一持久域，不接受 HOME、数据目录或远程路径。
    /// 参数：deadline 为原启动期限；返回：固定四槽容量域或拒绝，不修复已有权限。
    pub fn for_current_user(deadline: Instant) -> Result<Self, SlotError> {
        Self::from_host(
            crate::trusted_local_recovery_home::current_directory(deadline)?,
            deadline,
        )
    }
    /// 参数：directory 为可信宿主提供的持久目录句柄；deadline 为原准入绝对期限。
    /// 返回：当前用户独占的原容量域；不接受远程路径、不创建或修复目录。
    pub fn from_host(directory: File, deadline: Instant) -> Result<Self, SlotError> {
        let domain = Self { directory };
        domain.check_directory(deadline)?;
        Ok(domain)
    }

    /// 参数：deadline 为原准入期限；返回：原 native 锁预留，异常记录绝不重用。
    pub fn reserve(&self, deadline: Instant) -> Result<SlotReservation, SlotError> {
        self.check_directory(deadline)?;
        let mut unconfirmed = false;
        // 容量固定于本地部署协议，不允许请求提供名称、数量或新增随机容量域。
        for name in [
            c"supervisor_0.slot",
            c"supervisor_1.slot",
            c"supervisor_2.slot",
            c"supervisor_3.slot",
        ] {
            self.check_directory(deadline)?;
            let file = match crate::recovery_slot_open::open(&self.directory, name, deadline) {
                Ok(file) => file,
                Err(SlotError::Io(error)) => {
                    // helper 已捕获原 errno；诊断不能覆盖原系统错误。
                    let (entry, entry_errno) =
                        crate::recovery_slot_entry_diagnostic::observe(&self.directory, name);
                    // held 目录被移除后 fstat 仍可成功，但创建子项可能 ENOENT；只记录状态诊断，
                    // 不输出目录路径/身份，不重建容量域，也不将失败重试为新的出生授权。
                    let directory_links = self
                        .directory
                        .metadata()
                        .ok()
                        .map(|metadata| metadata.nlink());
                    #[cfg(target_os = "macos")]
                    let namespace = if error.raw_os_error() == Some(libc::ENOENT) {
                        held_path_state(&self.directory)
                    } else {
                        "not_enoent"
                    };
                    #[cfg(not(target_os = "macos"))]
                    let namespace = "unobserved";
                    eprintln!(
                        "diskgraph: recovery slot_stage=open failed errno={:?} directory_links={directory_links:?} namespace={namespace} entry={entry} entry_errno={entry_errno:?}",
                        error.raw_os_error()
                    );
                    return Err(error.into());
                }
                Err(error) => return Err(error),
            };
            let metadata = file.metadata().inspect_err(|_| {
                eprintln!("diskgraph: recovery slot_stage=metadata failed");
            })?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
                || metadata.mode() & 0o7777 != 0o600
            {
                return Err(SlotError::Unsupported);
            }
            // 新槽的目录项先持久化，随后原协议同步 RESERVED/ACTIVE；不能只 fsync 文件正文。
            self.directory.sync_all().inspect_err(|_| {
                eprintln!("diskgraph: recovery slot_stage=directory_sync failed");
            })?;
            self.check_directory(deadline)?;
            match SlotReservation::acquire(file, deadline) {
                Ok(reservation) => return Ok(reservation),
                Err(SlotError::Busy) => {}
                Err(SlotError::Unconfirmed) => unconfirmed = true,
                Err(error) => {
                    eprintln!("diskgraph: recovery slot_stage=reservation failed");
                    return Err(error);
                }
            }
        }
        if unconfirmed {
            Err(SlotError::Unconfirmed)
        } else {
            Err(SlotError::Busy)
        }
    }

    fn check_directory(&self, deadline: Instant) -> Result<(), SlotError> {
        if Instant::now() >= deadline {
            return Err(SlotError::Deadline);
        }
        let metadata = self.directory.metadata()?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o7777 != 0o700
        {
            return Err(SlotError::Unsupported);
        }
        if Instant::now() >= deadline {
            return Err(SlotError::Deadline);
        }
        Ok(())
    }
}

// 仅失败诊断：APFS 在目录被移除后仍可能报告非零 nlink，不能据此认定命名空间存在。
// 内核给出的路径只用于 lstat 比较，不打开正文、不输出路径，不参与授权或容量域恢复。
#[cfg(target_os = "macos")]
fn held_path_state(directory: &File) -> &'static str {
    let mut path = [0_u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_GETPATH, path.as_mut_ptr()) } < 0 {
        return "unknown";
    }
    let Ok(path) = std::ffi::CStr::from_bytes_until_nul(&path) else {
        return "unknown";
    };
    let mut named = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::lstat(path.as_ptr(), named.as_mut_ptr()) } < 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            "held_path_missing"
        } else {
            "unknown"
        };
    }
    let named = unsafe { named.assume_init() };
    match directory.metadata() {
        Ok(held) if held.dev() == named.st_dev as u64 && held.ino() == named.st_ino => {
            "held_path_same"
        }
        Ok(_) => "held_path_changed",
        Err(_) => "unknown",
    }
}

#[cfg(all(test, target_os = "macos"))]
#[test]
fn removed_held_directory_is_observed_without_trusting_its_link_count() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("held");
    std::fs::create_dir(&path).unwrap();
    let directory = File::open(&path).unwrap();
    assert_eq!(held_path_state(&directory), "held_path_same");
    std::fs::remove_dir(&path).unwrap();
    assert!(
        directory.metadata().is_ok(),
        "held metadata remains readable after removal"
    );
    assert_eq!(held_path_state(&directory), "held_path_missing");
}
