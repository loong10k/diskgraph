use super::linux_open::unique_mount;
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::fs::File;
use std::os::unix::fs::MetadataExt;

/// 持有原目录 FD 及其一次捕获的不可复用身份；来源：Rust FS-02 / Linux fstat、statx。
/// FD 始终存活以固定原 inode/mount，缓存不包含路径、mtime 或当前名称解析结果。
pub(super) struct LinuxDirectoryIdentity {
    file: File,
    device: u64,
    inode: u64,
    mount: u64,
}
impl LinuxDirectoryIdentity {
    /// 参数：已打开原目录的所有权及原扫描检查；返回：FD 与原 dev/inode/unique-mount。
    /// 捕获不是 namespace 原子快照，调用者仍须在整条链构造后重新验证当前路径。
    pub(super) fn capture(
        file: File,
        check: &dyn Fn() -> Result<(), Failure>,
    ) -> Result<Self, Failure> {
        check()?;
        let metadata = file.metadata().map_err(|_| Failure::Unavailable)?;
        check()?;
        let mount = unique_mount(&file)?;
        check()?;
        Ok(Self {
            file,
            device: metadata.dev(),
            inode: metadata.ino(),
            mount,
        })
    }
    /// 参数：无；返回：始终保留的原目录句柄借用，不重新按路径打开。
    pub(super) fn file(&self) -> &File {
        &self.file
    }
    /// 参数：本轮从当前父目录重新解析的目录和原扫描检查；返回：身份一致或固定失败。
    /// 每轮重新读取 current 的 dev/inode/unique-mount，不以缓存代替当前路线校验。
    pub(super) fn verify_current(
        &self,
        current: &File,
        check: &dyn Fn() -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        check()?;
        let metadata = current.metadata().map_err(|_| Failure::Unavailable)?;
        check()?;
        let mount = unique_mount(current)?;
        check()?;
        if self.device != metadata.dev() || self.inode != metadata.ino() || self.mount != mount {
            return Err(Failure::Conflict);
        }
        Ok(())
    }
}
