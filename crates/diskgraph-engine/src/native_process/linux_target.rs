use super::ProcessNativeSession;
use super::linux_metadata::capture;
use super::linux_open::{open_at, unique_mount};
use diskgraph_core::{IndexedFileEpoch, ProcessEvidenceFailureCode as Failure};
use std::ffi::CString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

/// 绑定扫描时强身份的 held 普通文件；来源：Linux D42，现场观察不能补旧索引缺失。
pub(super) struct LinuxTarget {
    root: File,
    root_name: CString,
    name: CString,
    pub(super) file: File,
    pub(super) device: u64,
    pub(super) inode: u64,
}
impl LinuxTarget {
    /// 参数：实际 scope 根、已索引相对路径、扫描 epoch、boot 与原账本；返回：只读元数据租约。
    pub(super) fn open(
        root: &Path,
        relative: &Path,
        expected: &IndexedFileEpoch,
        boot: &[u8; 36],
        session: &ProcessNativeSession<'_>,
    ) -> Result<Self, Failure> {
        session.admit(
            8192,
            1,
            root.as_os_str().len() as u64 + relative.as_os_str().len() as u64 + 256,
        )?;
        if !root.is_absolute()
            || relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Failure::Unsupported);
        }
        if !matches!(expected, IndexedFileEpoch::LinuxHandle { .. }) || expected.validate().is_err()
        {
            return Err(Failure::Unsupported);
        }
        let root_name =
            CString::new(root.as_os_str().as_bytes()).map_err(|_| Failure::Unsupported)?;
        let name =
            CString::new(relative.as_os_str().as_bytes()).map_err(|_| Failure::Unsupported)?;
        let root = open_at(
            libc::AT_FDCWD,
            &root_name,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        session.check()?;
        let file = open_at(
            root.as_raw_fd(),
            &name,
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0x08 | 0x04 | 0x02 | 0x01 | 0x20,
        )?;
        let observation = capture(&file, boot, &|| session.check())?;
        if observation.epoch() != expected {
            return Err(Failure::Conflict);
        }
        let IndexedFileEpoch::LinuxHandle { device, inode, .. } = expected else {
            return Err(Failure::Unsupported);
        };
        Ok(Self {
            root,
            root_name,
            name,
            file,
            device: *device,
            inode: *inode,
        })
    }
    /// 参数：原扫描 epoch、boot、同账本；返回：实际路径和 held 对象仍同身份，修改正文不制造新 epoch。
    pub(super) fn verify(
        &self,
        expected: &IndexedFileEpoch,
        boot: &[u8; 36],
        session: &ProcessNativeSession<'_>,
    ) -> Result<(), Failure> {
        session.admit(8192, 1, 256)?;
        let root = open_at(
            libc::AT_FDCWD,
            &self.root_name,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            0x04 | 0x02 | 0x20,
        )?;
        session.check()?;
        let a = self.root.metadata().map_err(|_| Failure::Unavailable)?;
        let b = root.metadata().map_err(|_| Failure::Unavailable)?;
        if a.dev() != b.dev()
            || a.ino() != b.ino()
            || unique_mount(&self.root)? != unique_mount(&root)?
        {
            return Err(Failure::Conflict);
        }
        session.check()?;
        let rebound = open_at(
            self.root.as_raw_fd(),
            &self.name,
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0x08 | 0x04 | 0x02 | 0x01 | 0x20,
        )?;
        let observation = capture(&rebound, boot, &|| session.check())?;
        if observation.epoch() != expected {
            return Err(Failure::Conflict);
        }
        Ok(())
    }
}
