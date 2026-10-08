use crate::recovery_slot::{SlotError, SlotReservation};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
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
            let raw = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC
                        | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if raw < 0 {
                // 先捕获原 errno；诊断输出不得改变返回的系统错误，也不包含用户路径。
                let error = std::io::Error::last_os_error();
                eprintln!("diskgraph: recovery slot_stage=open failed");
                return Err(error.into());
            }
            let file = unsafe { File::from_raw_fd(raw) };
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
