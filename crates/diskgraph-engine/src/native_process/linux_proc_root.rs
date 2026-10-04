use super::linux_open::{filesystem, last_error, open_at, unique_mount};
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;

/// 保留实际 procfs 挂载及本 boot 域；来源：Linux proc(5)、STATX_MNT_ID_UNIQUE。
/// 不把可复用 namespace inode 或普通 mount ID 当作永久进程身份。
pub(super) struct LinuxProcRoot {
    pub(super) file: File,
    pub(super) boot: [u8; 36],
    pub(super) domain: [u8; 32],
}
impl LinuxProcRoot {
    /// 参数：原任务检查；返回：已验证 procfs 的持有租约及固定域。
    pub(super) fn open(check: &dyn Fn() -> Result<(), Failure>) -> Result<Self, Failure> {
        check()?;
        let file = open_at(
            libc::AT_FDCWD,
            c"/proc",
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        check()?;
        if filesystem(&file)? != 0x9fa0 {
            return Err(Failure::Unsupported);
        }
        check()?;
        let mount = unique_mount(&file)?;
        check()?;
        // 唯一允许正文式读取的是已验证 procfs 的固定内核元数据，不接受客户端路径。
        let mut boot_file = open_at(
            file.as_raw_fd(),
            c"sys/kernel/random/boot_id",
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0x08 | 0x04 | 0x02,
        )?;
        if filesystem(&boot_file)? != 0x9fa0 {
            return Err(Failure::Unsupported);
        }
        check()?;
        let mut bytes = [0_u8; 38];
        let count = boot_file.read(&mut bytes).map_err(|_| last_error())?;
        check()?;
        if count != 37
            || bytes[36] != b'\n'
            || !bytes[..36].iter().enumerate().all(|(i, b)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    *b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
        {
            return Err(Failure::Unsupported);
        }
        let boot: [u8; 36] = bytes[..36].try_into().map_err(|_| Failure::InternalError)?;
        let mut h = Sha256::new();
        h.update(b"diskgraph-linux-procfs-boot-mount-v1\0");
        h.update(boot);
        h.update(mount.to_le_bytes());
        Ok(Self {
            file,
            boot,
            domain: h.finalize().into(),
        })
    }
}
